#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod audio;
mod menu;
mod music;
mod net;
mod prefs;
mod physics;
mod render;
mod rules;

use ai::{AiShot, Difficulty};
use audio::Sounds;
use macroquad::prelude::*;
use net::{Msg, Peer};
use physics::{
    BallState, DT, Event, HALF_L, HALF_W, HEAD_SPOT, R, Rng, ShotLog, V2, World, cast_cue,
};
use render::{Cam, Renderer, THEMES, at};
use rules::{Choice, Group, Mode, Rules, ShotCall, ShotContext};
use menu::{Lobby, MenuAction, MenuState};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

const GOLD: Color = Color::new(0.96, 0.76, 0.33, 1.0);
const INK: Color = Color::new(0.93, 0.94, 0.96, 1.0);
const DIM: Color = Color::new(0.62, 0.65, 0.70, 1.0);
const PANEL: Color = Color::new(0.055, 0.062, 0.078, 0.90);
const EDGE: Color = Color::new(1.0, 1.0, 1.0, 0.11);
const POCKET_NAMES: [&str; 6] = [
    "bottom-left",
    "top-left",
    "bottom-right",
    "top-right",
    "bottom-middle",
    "top-middle",
];

fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn has_arg(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn windowed_start() -> bool {
    has_arg("--windowed")
        || arg_value("--shot").is_some()
        || has_arg("--host")
        || arg_value("--join").is_some()
}

fn conf() -> Conf {
    Conf {
        window_title: "Breakpoint Billiards".to_owned(),
        window_width: 1280,
        window_height: 720,
        high_dpi: true,
        fullscreen: !windowed_start(),
        sample_count: 4,
        window_resizable: true,
        icon: Some(macroquad::miniquad::conf::Icon {
            small: *include_bytes!("../assets/icon-16.rgba"),
            medium: *include_bytes!("../assets/icon-32.rgba"),
            big: *include_bytes!("../assets/icon-64.rgba"),
        }),
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Opponent {
    Human,
    Ai(Difficulty),
    Online,
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    /// Shooter-side view while aiming, automatic table view during the shot.
    Cue,
    /// A camera the player positions anywhere around the table.
    Free,
    Top,
}

/// Who is in control of a player's turn on this computer.
#[derive(Clone, Copy, PartialEq)]
enum Ctl {
    Human,
    Computer(Difficulty),
    Remote,
}

struct Settings {
    mode: Mode,
    opponent: Opponent,
    /// Computer level remembered while another opponent is chosen.
    level: Difficulty,
    view: ViewMode,
    fullscreen: bool,
    guide: u8,
    pocket_hints: bool,
    /// Testing aid: the computer plays the local player's turns too.
    autoplay: bool,
    log: Option<String>,
}

struct Sfx(Option<Sounds>);

impl Sfx {
    fn with(&self, f: impl FnOnce(&Sounds)) {
        if let Some(s) = &self.0 {
            f(s);
        }
    }
    fn click(&self) {
        self.with(|s| s.click());
    }
}

struct Layout {
    ui: f32,
    sw: f32,
    sh: f32,
    top: f32,
    spin_c: Vec2,
    spin_r: f32,
}

fn layout() -> Layout {
    let (sw, sh) = (screen_width(), screen_height());
    let ui = (sh / 720.0).min(sw / 1180.0).clamp(0.6, 3.0);
    let spin_r = 31.0 * ui;
    Layout {
        ui,
        sw,
        sh,
        top: 86.0 * ui,
        spin_c: vec2(sw - 22.0 * ui - spin_r, sh - 14.0 * ui - spin_r),
        spin_r,
    }
}

fn mouse() -> Vec2 {
    let (x, y) = mouse_position();
    vec2(x, y)
}

/// Online shot clock, in seconds. Professional pool (Matchroom's World
/// Nineball Tour) allows 30 seconds a shot, 60 for the shot after the break
/// and one 30-second extension per player per rack; the same is used here,
/// with the break itself also given 60.
const SHOT_CLOCK: f32 = 30.0;
const BREAK_CLOCK: f32 = 60.0;
const EXTENSION: f32 = 30.0;

fn power_to_speed(p: f32) -> f64 {
    0.35 + 8.65 * (p.clamp(0.0, 1.0) as f64).powf(1.6)
}

fn speed_to_power(s: f64) -> f32 {
    (((s - 0.35) / 8.65).clamp(0.0, 1.0)).powf(1.0 / 1.6) as f32
}

fn angle_diff(from: f64, to: f64) -> f64 {
    let mut d = (to - from) % std::f64::consts::TAU;
    if d > std::f64::consts::PI {
        d -= std::f64::consts::TAU;
    }
    if d < -std::f64::consts::PI {
        d += std::f64::consts::TAU;
    }
    d
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

struct ButtonStyle {
    selected: bool,
    primary: bool,
}

const PLAIN: ButtonStyle = ButtonStyle {
    selected: false,
    primary: false,
};
const PRIMARY: ButtonStyle = ButtonStyle {
    selected: false,
    primary: true,
};

fn button(r: &Renderer, sfx: &Sfx, rect: Rect, label: &str, size: f32, style: ButtonStyle) -> bool {
    let hover = rect.contains(mouse());
    let (fill, border, text) = if style.primary {
        let k = if hover { 1.0 } else { 0.9 };
        (
            Color::new(GOLD.r * k, GOLD.g * k, GOLD.b * k, 1.0),
            Color::new(1.0, 1.0, 1.0, 0.35),
            Color::new(0.08, 0.06, 0.02, 1.0),
        )
    } else if style.selected {
        (Color::new(GOLD.r, GOLD.g, GOLD.b, 0.20), GOLD, INK)
    } else if hover {
        (
            Color::new(1.0, 1.0, 1.0, 0.13),
            Color::new(1.0, 1.0, 1.0, 0.30),
            INK,
        )
    } else {
        (Color::new(1.0, 1.0, 1.0, 0.055), EDGE, DIM)
    };
    r.panel(rect, rect.h * 0.28, fill, border, 1.3);
    r.text_center(
        label,
        rect.x + rect.w / 2.0,
        rect.y + rect.h / 2.0,
        size,
        text,
        style.primary || style.selected,
    );
    let clicked = hover && is_mouse_button_pressed(MouseButton::Left);
    if clicked {
        sfx.click();
    }
    clicked
}

fn rack(world: &mut World, mode: Mode, seed: u64) {
    let mut rng = Rng::new(seed);
    match mode {
        Mode::NineBall => physics::rack_nine_ball(world, &mut rng),
        _ => physics::rack_eight_ball(world, &mut rng),
    }
}

fn mode_code(m: Mode) -> u8 {
    match m {
        Mode::EightBall => 0,
        Mode::NineBall => 1,
        Mode::Practice => 2,
    }
}

fn snapshot(world: &World) -> Vec<(u8, bool, f64, f64)> {
    world
        .balls
        .iter()
        .map(|b| (b.id, b.on_table(), b.pos.x, b.pos.y))
        .collect()
}

fn state_hash(world: &World) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for (id, on, x, y) in snapshot(world) {
        for v in [id as u64, on as u64, x.to_bits(), y.to_bits()] {
            h = (h ^ v).wrapping_mul(0x100000001b3);
        }
    }
    h
}

enum Phase {
    Aim,
    /// The button is held and the shot is being drawn back. `start` is where
    /// the mouse was pressed, `held` the seconds since.
    Pull {
        start: Vec2,
        held: f32,
    },
    DragCue,
    Stroke {
        t: f32,
        from: f64,
        speed: f64,
    },
    Rolling,
    /// The remote shooter's table has stopped; waiting for its final state.
    WaitState {
        t: f32,
    },
    AiThink {
        t: f32,
    },
    AiMove {
        shot: AiShot,
        t: f32,
        from_angle: f64,
        from_pos: V2,
    },
    RemoteAim,
    Decide,
    Over,
}

struct Game {
    world: World,
    rules: Rules,
    ctx: ShotContext,
    phase: Phase,
    opponent: Opponent,
    autoplay: bool,
    /// Which player this computer plays when online.
    local: usize,
    net: Option<Peer>,
    disconnected: bool,
    remote_state: Option<(Vec<(u8, bool, f64, f64)>, ShotLog)>,
    /// The host asked for a new game: seed and breaker.
    restart: Option<(u64, usize)>,
    /// Messages that arrived before this side was ready for them.
    inbox: std::collections::VecDeque<Msg>,
    /// Where the remote player's cue is heading: aim and power. The local
    /// copy glides there so updates arriving a few times a second look smooth.
    remote_cue: Option<(f64, f32)>,
    shooter_local: bool,
    cue_sent: f64,
    /// Online shot clock: seconds left for the player at the table.
    clock: f32,
    /// Whether each player has used their one extension this rack.
    extended: [bool; 2],
    /// The last shot was a break, so the next one gets the longer clock.
    after_break: bool,
    /// Chat lines: mine?, text, time received.
    chat: Vec<(bool, String, f64)>,
    /// The chat line being typed, while the chat box is open.
    typing: Option<String>,
    aim: f64,
    power: f32,
    gap: f64,
    /// Tip offset: x = side (right positive), y = top (follow positive).
    spin: Vec2,
    spin_drag: bool,
    call: ShotCall,
    manual_ball: Option<u8>,
    manual_pocket: Option<usize>,
    rx: Option<Receiver<AiShot>>,
    toast: String,
    toast_at: f64,
    acc: f64,
    rng: Rng,
    seed: u64,
    racks: u64,
    shots: u32,
    log_path: Option<String>,
    last_mouse: Vec2,
    paused: bool,
    view: ViewMode,
    cam: Cam,
    /// Orbit offset from the shooter-side camera, relative to the cue.
    yaw: f32,
    pitch: f32,
    dist: f32,
    /// Free camera: angles, distance and the point it looks at.
    fyaw: f32,
    fpitch: f32,
    fdist: f32,
    focus: Vec2,
    /// Seconds since the waiting player started tapping the table.
    tap: f32,
    /// Ball positions when the shot was struck, for judging how good it was.
    before: Vec<(u8, V2)>,
    /// Aim guide: 0 full, 1 short, 2 off.
    guide: u8,
    pocket_hints: bool,
    /// Brief cue follow-through after contact, in seconds.
    cue_follow: f32,
    cue_follow_origin: V2,
    rack_sound: bool,
}

enum Action {
    None,
    Restart,
    Menu,
    Quit,
}

impl Game {
    fn new(
        settings: &Settings,
        breaker: usize,
        seed: u64,
        net: Option<Peer>,
        local: usize,
    ) -> Game {
        let mut world = World::new();
        rack(&mut world, settings.mode, seed);
        let names = match (settings.mode, settings.opponent) {
            (Mode::Practice, _) => ["Practice".to_string(), "Practice".to_string()],
            (_, Opponent::Human) => ["Player 1".to_string(), "Player 2".to_string()],
            (_, Opponent::Ai(d)) => ["You".to_string(), format!("Computer ({})", d.name())],
            (_, Opponent::Online) => {
                if local == 0 {
                    ["You".to_string(), "Opponent".to_string()]
                } else {
                    ["Opponent".to_string(), "You".to_string()]
                }
            }
        };
        let first = if settings.mode == Mode::Practice {
            0
        } else {
            breaker
        };
        let rules = Rules::new(settings.mode, names, first);
        let ctx = rules.context(&world);
        let mut g = Game {
            world,
            rules,
            ctx,
            phase: Phase::Aim,
            opponent: if settings.mode == Mode::Practice {
                Opponent::Human
            } else {
                settings.opponent
            },
            autoplay: settings.autoplay,
            local,
            net,
            disconnected: false,
            remote_state: None,
            restart: None,
            inbox: std::collections::VecDeque::new(),
            remote_cue: None,
            shooter_local: true,
            cue_sent: 0.0,
            clock: BREAK_CLOCK,
            extended: [false; 2],
            after_break: false,
            chat: Vec::new(),
            typing: None,
            aim: 0.0,
            power: 0.0,
            gap: 0.0,
            spin: Vec2::ZERO,
            spin_drag: false,
            call: ShotCall::default(),
            manual_ball: None,
            manual_pocket: None,
            rx: None,
            toast: String::new(),
            toast_at: -100.0,
            acc: 0.0,
            rng: Rng::new(seed ^ 0x5EED),
            seed,
            racks: 1,
            shots: 0,
            log_path: settings.log.clone(),
            last_mouse: mouse(),
            paused: false,
            view: settings.view,
            cam: Cam::overview(),
            yaw: 0.0,
            pitch: 0.29,
            dist: 1.65,
            fyaw: std::f32::consts::FRAC_PI_2,
            fpitch: 0.68,
            fdist: 2.9,
            focus: Vec2::ZERO,
            tap: 10.0,
            before: Vec::new(),
            guide: settings.guide,
            pocket_hints: settings.pocket_hints,
            cue_follow: 0.0,
            cue_follow_origin: V2::ZERO,
            rack_sound: true,
        };
        g.say(match settings.mode {
            Mode::EightBall => "8-Ball - break from behind the line",
            Mode::NineBall => "9-Ball - hit the lowest ball first",
            Mode::Practice => "Practice - no rules, take your time",
        });
        g.begin_turn();
        g
    }

    fn say(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        // Referee messages are written in the third person; fix them up when
        // the player they name is "You".
        let mut s = s.to_string();
        for (a, b) in [
            ("You to shoot", "Your shot"),
            ("You shoots", "You shoot"),
            ("You sinks", "You sink"),
            ("You takes", "You take"),
            ("You keeps", "You keep"),
            ("You is on", "You are on"),
            ("You wins", "You win"),
            ("You decides", "You decide"),
            ("for You", "for you"),
        ] {
            s = s.replace(a, b);
        }
        self.toast = s;
        self.toast_at = get_time();
    }

    fn ctl_of(&self, player: usize) -> Ctl {
        let me = if self.autoplay {
            Ctl::Computer(Difficulty::Medium)
        } else {
            Ctl::Human
        };
        match self.opponent {
            Opponent::Human => me,
            Opponent::Ai(d) if player == 1 => Ctl::Computer(d),
            Opponent::Ai(_) => me,
            Opponent::Online if player == self.local => me,
            Opponent::Online => Ctl::Remote,
        }
    }

    fn send(&mut self, msg: Msg) {
        if let Some(p) = self.net.as_mut() {
            p.send(&msg);
        }
    }

    /// The online shot clock ran out for the player at the table.
    fn time_foul(&mut self, sfx: &Sfx) {
        let was_break = self.rules.break_shot;
        let out = self.rules.time_foul();
        sfx.with(|s| s.foul());
        self.rules.apply(&mut self.world, &out);
        if was_break && out.winner.is_none() {
            // Nobody has broken yet: the other player breaks instead.
            self.rules.break_shot = true;
            self.rules.kitchen = true;
        }
        self.power = 0.0;
        self.gap = 0.0;
        self.say(&out.message);
        if out.winner.is_some() {
            sfx.with(|s| s.chime());
            self.phase = Phase::Over;
        } else {
            self.begin_turn();
        }
    }

    /// Use this rack's one time extension, if it is still available.
    fn extend_clock(&mut self) {
        let me = self.rules.current;
        if self.extended[me] {
            return;
        }
        self.extended[me] = true;
        self.clock += EXTENSION;
        if self.ctl_of(me) == Ctl::Remote {
            self.say("Opponent takes a time extension");
        } else {
            self.send(Msg::Extend);
            self.say("Extension: 30 more seconds");
        }
    }

    /// Resolve re-racks and decisions, then hand the table to whoever shoots.
    fn begin_turn(&mut self) {
        if let Some(breaker) = self.rules.rerack.take() {
            rack(
                &mut self.world,
                self.rules.mode,
                self.seed.wrapping_add(self.racks),
            );
            self.racks += 1;
            self.rules.reset_rack(breaker);
            self.extended = [false; 2];
            self.rack_sound = true;
            self.say("New rack");
        }
        if let Some(d) = self.rules.pending.clone() {
            if let Ctl::Computer(_) = self.ctl_of(d.by) {
                let c = ai::choose(&d);
                self.decide(c, true);
            } else {
                self.phase = Phase::Decide;
            }
            return;
        }
        // The break, and the shot that follows it, get the longer clock.
        let long = self.rules.break_shot || self.after_break;
        self.after_break = false;
        self.clock = if long { BREAK_CLOCK } else { SHOT_CLOCK };
        // Testing aid: BPB_TEST_CLOCK shortens every clock to that many seconds.
        if let Some(t) = std::env::var("BPB_TEST_CLOCK").ok().and_then(|t| t.parse::<f32>().ok()) {
            self.clock = t.clamp(1.0, BREAK_CLOCK);
        }
        self.power = 0.0;
        self.gap = 0.0;
        self.cue_follow = 0.0;
        self.spin = Vec2::ZERO;
        self.spin_drag = false;
        self.call = ShotCall::default();
        self.manual_ball = None;
        self.manual_pocket = None;
        self.ctx = self.rules.context(&self.world);
        // Start by pointing at something sensible; the camera tracks the cue.
        let cue = self.world.cue().pos;
        if let Some(b) = self.ctx.legal.first().and_then(|&id| self.world.ball(id)) {
            self.aim = (b.pos - cue).angle();
        }
        self.yaw = 0.0;
        match self.ctl_of(self.rules.current) {
            Ctl::Computer(diff) => {
                let (tx, rx) = channel();
                let world = self.world.clone();
                let rules = self.rules.clone();
                let seed = self.rng.next_u64();
                std::thread::spawn(move || {
                    let _ = tx.send(ai::plan(&world, &rules, diff, seed));
                });
                self.rx = Some(rx);
                self.phase = Phase::AiThink { t: 0.0 };
            }
            Ctl::Remote => self.phase = Phase::RemoteAim,
            Ctl::Human => self.phase = Phase::Aim,
        }
    }

    /// Apply a choice. `local` choices are also sent to the other computer.
    fn decide(&mut self, choice: Choice, local: bool) {
        if self.rules.pending.is_none() {
            return;
        }
        if local {
            self.send(Msg::Choice(choice.index() as u8));
        }
        self.rules.choose(&mut self.world, choice);
        self.say(choice.label());
        self.begin_turn();
    }

    fn fire(&mut self, speed: f64, sfx: &Sfx) {
        self.after_break = self.rules.break_shot;
        self.cue_follow_origin = self.world.cue().pos;
        self.shooter_local = self.ctl_of(self.rules.current) != Ctl::Remote;
        if self.shooter_local {
            let c = self.world.cue().pos;
            let (side, top) = (self.spin.x as f64, self.spin.y as f64);
            self.send(Msg::Shot {
                x: c.x,
                y: c.y,
                angle: self.aim,
                speed,
                side,
                top,
                call: self.call,
            });
        }
        self.ctx = self.rules.context(&self.world);
        self.before = self
            .world
            .balls
            .iter()
            .filter(|b| b.on_table())
            .map(|b| (b.id, b.pos))
            .collect();
        self.world.begin_shot();
        self.world
            .strike(self.aim, speed, self.spin.x as f64, self.spin.y as f64);
        self.rules.ball_in_hand = false;
        self.acc = 0.0;
        self.gap = 0.0;
        self.cue_follow = 0.12;
        self.phase = Phase::Rolling;
        sfx.with(|s| s.cue_strike(speed));
    }

    fn finish_shot(&mut self, sfx: &Sfx) {
        let out = self
            .rules
            .judge(&self.ctx, &self.call, &self.world.log, &self.world);
        // A long pot, a combination or two balls in one shot earns a tap on
        // the table from the other player.
        let shooter = self.rules.current;
        let log = &self.world.log;
        let kept = out.foul.is_none()
            && !log.potted.is_empty()
            && (out.next_player == shooter || out.winner == Some(shooter));
        let great = kept && self.rules.mode != Mode::Practice && {
            let start = |id: u8| self.before.iter().find(|b| b.0 == id).map(|b| b.1);
            let reach = match (
                start(0),
                log.first_hit.and_then(start),
                start(log.potted[0]),
                log.pockets.first(),
            ) {
                (Some(c), Some(f), Some(b), Some(&p)) => {
                    (f - c).len() + (self.world.pockets[p].center - b).len()
                }
                _ => 0.0,
            };
            log.potted.len() >= 2 || log.first_hit != Some(log.potted[0]) || reach > 1.55
        };
        if out.foul.is_some() {
            sfx.with(|s| s.foul());
        }
        self.rules.apply(&mut self.world, &out);
        if out.rerack {
            rack(
                &mut self.world,
                self.rules.mode,
                self.seed.wrapping_add(self.racks),
            );
            self.racks += 1;
            self.rules.break_shot = true;
            self.rules.ball_in_hand = true;
            self.rules.kitchen = true;
        }
        self.shots += 1;
        if let Some(path) = &self.log_path {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    f,
                    "shot {} hash {:016x} next {} winner {:?} msg {}",
                    self.shots,
                    state_hash(&self.world),
                    self.rules.current,
                    self.rules.winner,
                    out.message
                );
            }
        }
        let mut msg = out.message.clone();
        if great {
            self.tap = 0.0;
            msg = format!("Great shot!  {msg}");
        }
        if self.rules.rerack.is_some() && out.decision.is_none() {
            msg = "Stalemate - the rack is played again".into();
        }
        self.say(&msg);
        if out.winner.is_some() {
            sfx.with(|s| s.chime());
            self.phase = Phase::Over;
        } else {
            self.begin_turn();
        }
    }

    /// Try to move the cue ball to `p` while it is in hand.
    fn try_place(&mut self, p: V2) {
        let max_x = if self.rules.kitchen {
            HEAD_SPOT.x - 0.0005
        } else {
            HALF_L - R - 0.002
        };
        let p = physics::v2(
            p.x.clamp(-(HALF_L - R - 0.002), max_x),
            p.y.clamp(-(HALF_W - R - 0.002), HALF_W - R - 0.002),
        );
        if self.world.spot_free(p, 0) {
            self.world.balls[0].pos = p;
        }
    }

    /// Work out the call: obvious shots are called automatically, anything
    /// else is whatever the player marked by hand.
    fn update_call(&mut self) {
        let eight_ball = self.rules.mode == Mode::EightBall && !self.rules.break_shot;
        let mut call = ShotCall {
            safety: self.call.safety && eight_ball,
            push_out: self.call.push_out && self.ctx.can_push,
            ..ShotCall::default()
        };
        if eight_ball && !call.safety {
            let cue = self.world.cue().pos;
            let d = V2::from_angle(self.aim);
            let hit = cast_cue(&self.world, cue, d);
            let mut auto: (Option<u8>, Option<usize>) = (None, None);
            if let Some(t) = hit.ball.and_then(|id| self.world.ball(id)) {
                if self.ctx.legal.contains(&t.id) {
                    let n = (t.pos - hit.point).norm();
                    let mut best = (0.985, None);
                    for (i, p) in self.world.pockets.iter().enumerate() {
                        let c = (p.aim - t.pos).norm().dot(n);
                        if c > best.0 {
                            best = (c, Some(i));
                        }
                    }
                    auto = (Some(t.id), best.1);
                }
            }
            call.ball = self.manual_ball.or(auto.0);
            call.pocket = self
                .manual_pocket
                .or(if call.ball == auto.0 { auto.1 } else { None });
        }
        self.call = call;
    }

    fn call_text(&self) -> String {
        if self.call.push_out {
            return "Push out called  (P to cancel)".into();
        }
        if self.rules.mode != Mode::EightBall || self.rules.break_shot {
            return if self.ctx.can_push {
                "P: call a push out".into()
            } else {
                String::new()
            };
        }
        if self.call.safety {
            return "Safety called  (S to cancel)".into();
        }
        match (self.call.ball, self.call.pocket) {
            (Some(b), Some(p)) => format!("Call: {b}-ball, {} pocket", POCKET_NAMES[p]),
            (Some(b), None) => format!("Call: {b}-ball - click a pocket to call it"),
            _ => "No call - click a pocket, right-click a ball, or S for safety".into(),
        }
    }

    fn player_cam(&self) -> Cam {
        let focus = at(self.world.cue().pos, R as f32);
        // About forty degrees off the cue line keeps the shaft visible beside
        // the ball, like a player leaning in to sight a shot.
        let orbit = self.aim as f32 + 0.72 + self.yaw;
        let (sy, cy) = orbit.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (ay, ax) = (self.aim as f32).sin_cos();
        Cam {
            pos: focus + vec3(-cy * cp, -sy * cp, sp) * self.dist,
            target: focus + vec3(ax, ay, 0.0) * 0.29 + vec3(0.0, 0.0, 0.025),
            fov: 45.0_f32.to_radians(),
        }
    }

    fn free_cam(&self) -> Cam {
        let focus = self.focus.extend(0.0);
        let (sy, cy) = self.fyaw.sin_cos();
        let (sp, cp) = self.fpitch.sin_cos();
        Cam {
            pos: focus + vec3(-cy * cp, -sy * cp, sp) * self.fdist,
            target: focus,
            fov: 42.0_f32.to_radians(),
        }
    }

    fn aiming(&self) -> bool {
        matches!(
            self.phase,
            Phase::Aim | Phase::Pull { .. } | Phase::DragCue | Phase::Stroke { .. }
        )
    }

    /// Jump the free camera to a named angle.
    fn preset(&mut self, name: &str, yaw: f32, pitch: f32, dist: f32) {
        self.view = ViewMode::Free;
        self.fyaw = yaw;
        self.fpitch = pitch;
        self.fdist = dist;
        self.focus = Vec2::ZERO;
        self.say(name);
    }

    /// Camera controls work at any time, including while balls are rolling
    /// and during the opponent's turn.
    fn camera_input(&mut self, delta: Vec2) {
        use std::f32::consts::{FRAC_PI_2, PI};
        let keys = [
            (KeyCode::Key2, "Side view", FRAC_PI_2, 0.68, 2.9),
            (KeyCode::Key4, "Far side view", -FRAC_PI_2, 0.68, 2.9),
            (KeyCode::Key5, "Head end view", 0.0, 0.52, 3.3),
            (KeyCode::Key6, "Foot end view", PI, 0.52, 3.3),
            (KeyCode::Key7, "Corner view", 0.62, 0.60, 3.3),
            (KeyCode::Key8, "Low rail view", FRAC_PI_2, 0.20, 2.5),
        ];
        for (key, name, yaw, pitch, dist) in keys {
            if is_key_pressed(key) {
                self.preset(name, yaw, pitch, dist);
            }
        }
        if is_key_pressed(KeyCode::Key1) {
            self.view = ViewMode::Cue;
            self.yaw = 0.0;
            self.say("Shooter view");
        }
        if is_key_pressed(KeyCode::Key3) {
            self.view = ViewMode::Top;
            self.say("Top view");
        }
        if is_key_pressed(KeyCode::V) || is_key_pressed(KeyCode::Tab) {
            let (next, name) = match self.view {
                ViewMode::Cue => (ViewMode::Free, "Table orbit"),
                ViewMode::Free => (ViewMode::Top, "Top view"),
                ViewMode::Top => (ViewMode::Cue, "Shooter view"),
            };
            self.view = next;
            self.say(name);
        }
        if self.view == ViewMode::Top || self.spin_drag {
            return;
        }
        let shooter_view = self.view == ViewMode::Cue && self.aiming();
        if is_mouse_button_down(MouseButton::Right) {
            if shooter_view {
                self.yaw -= delta.x * 0.006;
                self.pitch = (self.pitch + delta.y * 0.004).clamp(0.15, 0.85);
            } else {
                self.fyaw -= delta.x * 0.006;
                self.fpitch = (self.fpitch + delta.y * 0.004).clamp(0.10, 1.45);
            }
        }
        if is_mouse_button_down(MouseButton::Middle) && delta.length() > 0.0 {
            // Slide the point the free camera looks at across the table.
            self.view = ViewMode::Free;
            let (sy, cy) = self.fyaw.sin_cos();
            let k = 0.0016 * self.fdist;
            self.focus += vec2(sy, -cy) * (-delta.x * k) + vec2(cy, sy) * (delta.y * k);
            self.focus = self.focus.clamp(vec2(-1.4, -0.8), vec2(1.4, 0.8));
        }
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            let k = 1.0 - wheel.signum() * 0.10;
            if shooter_view {
                self.dist = (self.dist * k).clamp(1.0, 3.4);
            } else {
                self.fdist = (self.fdist * k).clamp(0.7, 6.0);
            }
        }
    }

    fn update_camera(&mut self, dt: f32, lay: &Layout) {
        let want = match self.view {
            ViewMode::Top => Cam::top(lay.sw / lay.sh),
            ViewMode::Cue if self.aiming() => self.player_cam(),
            _ => self.free_cam(),
        };
        let response = if self.view == ViewMode::Cue && self.aiming() {
            13.0
        } else {
            6.0
        };
        self.cam = self.cam.lerp(want, 1.0 - (-dt * response).exp());
    }

    fn network(&mut self, sfx: &Sfx) {
        let Some(peer) = self.net.as_mut() else {
            return;
        };
        self.inbox.extend(peer.poll());
        let alive = peer.alive;
        // Handle messages in order; one that is early stays queued until
        // this side has caught up (for example while its balls still roll).
        while let Some(m) = self.inbox.pop_front() {
            let ready = self.ctl_of(self.rules.current) == Ctl::Remote
                && matches!(self.phase, Phase::RemoteAim);
            match m {
                Msg::Cue {
                    x,
                    y,
                    aim,
                    power,
                    side,
                    top,
                } => {
                    if ready {
                        let p = physics::v2(x, y);
                        if World::in_bounds(p) && self.world.spot_free(p, 0) {
                            self.world.balls[0].pos = p;
                        }
                        if self.remote_cue.is_none() {
                            self.aim = aim;
                        }
                        self.remote_cue = Some((aim, power.clamp(0.0, 1.0) as f32));
                        self.spin = vec2(side.clamp(-1.0, 1.0) as f32, top.clamp(-1.0, 1.0) as f32);
                    }
                }
                Msg::Shot {
                    x,
                    y,
                    angle,
                    speed,
                    side,
                    top,
                    call,
                } => {
                    if !ready {
                        self.inbox.push_front(Msg::Shot {
                            x,
                            y,
                            angle,
                            speed,
                            side,
                            top,
                            call,
                        });
                        break;
                    }
                    let p = physics::v2(x, y);
                    if World::in_bounds(p) && self.world.spot_free(p, 0) {
                        self.world.balls[0].pos = p;
                    }
                    self.aim = angle;
                    self.spin = vec2(side.clamp(-1.0, 1.0) as f32, top.clamp(-1.0, 1.0) as f32);
                    self.call = call;
                    self.fire(speed.clamp(0.1, 9.0), sfx);
                }
                Msg::State(balls, log) => {
                    if self.remote_state.is_some() {
                        self.inbox.push_front(Msg::State(balls, log));
                        break;
                    }
                    self.remote_state = Some((balls, log));
                }
                Msg::Choice(c) => {
                    let theirs = matches!(self.phase, Phase::Decide)
                        && self
                            .rules
                            .pending
                            .as_ref()
                            .map_or(false, |d| self.ctl_of(d.by) == Ctl::Remote);
                    if !theirs {
                        self.inbox.push_front(Msg::Choice(c));
                        break;
                    }
                    if let Some(&choice) = Choice::ALL.get(c as usize) {
                        self.decide(choice, false);
                    }
                }
                Msg::Start { seed, first, .. } => {
                    self.restart = Some((seed, (first as usize).min(1)))
                }
                Msg::Chat(text) => {
                    self.chat.push((false, text, get_time()));
                    if self.chat.len() > 50 {
                        self.chat.remove(0);
                    }
                    sfx.click();
                }
                Msg::Timeout => {
                    if !ready {
                        self.inbox.push_front(Msg::Timeout);
                        break;
                    }
                    self.time_foul(sfx);
                }
                Msg::Extend => {
                    if ready {
                        self.extend_clock();
                    }
                }
                Msg::Bye => {}
            }
        }
        if !alive && !self.disconnected {
            self.disconnected = true;
            self.net = None;
            self.say("Opponent disconnected");
        }
    }

    fn update(&mut self, dt: f32, lay: &Layout, sfx: &Sfx, frozen_input: bool) {
        self.network(sfx);
        // Chat: Enter opens the box, Enter sends, Esc closes. While it is
        // open the keyboard belongs to the chat, not the game.
        if self.net.is_none() {
            self.typing = None;
        } else if let Some(mut text) = self.typing.take() {
            while let Some(c) = get_char_pressed() {
                if !c.is_control() && text.chars().count() < net::CHAT_MAX {
                    text.push(c);
                }
            }
            if is_key_pressed(KeyCode::Backspace) {
                text.pop();
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                let text = net::clean_chat(&text);
                if !text.is_empty() {
                    self.send(Msg::Chat(text.clone()));
                    self.chat.push((true, text, get_time()));
                }
            } else if !is_key_pressed(KeyCode::Escape) {
                self.typing = Some(text);
            }
        } else if !frozen_input
            && !matches!(self.phase, Phase::Over)
            && (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter))
        {
            while get_char_pressed().is_some() {}
            self.typing = Some(String::new());
        }
        // Testing aid: BPB_TEST_CHAT sends one chat line as the match starts.
        if self.net.is_some() && self.chat.is_empty() && self.shots == 0 && self.clock < BREAK_CLOCK - 2.0 {
            if let Ok(text) = std::env::var("BPB_TEST_CHAT") {
                let text = net::clean_chat(&text);
                if !text.is_empty() {
                    self.send(Msg::Chat(text.clone()));
                    self.chat.push((true, text, get_time()));
                }
            }
        }
        // Online shot clock. Each game keeps time for its own player and
        // tells the other when it runs out; the other side only displays it.
        if self.net.is_some() {
            let mine = matches!(self.phase, Phase::Aim | Phase::Pull { .. } | Phase::DragCue);
            if mine || matches!(self.phase, Phase::RemoteAim) {
                let before = self.clock;
                self.clock = (self.clock - dt).max(0.0);
                if mine {
                    if self.clock <= 5.0 && self.clock > 0.0 && before.ceil() != self.clock.ceil() {
                        sfx.click();
                    }
                    if is_key_pressed(KeyCode::E) && self.typing.is_none() && !frozen_input {
                        self.extend_clock();
                    }
                    if self.clock <= 0.0 {
                        self.send(Msg::Timeout);
                        self.time_foul(sfx);
                    }
                }
            }
        }
        // While the chat box is open the game ignores its own hotkeys.
        let frozen_input = frozen_input || self.typing.is_some();
        let m = mouse();
        let delta = m - self.last_mouse;
        let moved = delta.length() > 0.5;
        self.last_mouse = m;
        let cue = self.world.cue().pos;
        let cs = self.cam.to_screen(cue);
        let in_spin = (m - lay.spin_c).length() <= lay.spin_r * 1.15;
        let ball_px = self.cam.px_per_m(at(cue, R as f32)) * R as f32;

        if !frozen_input {
            self.camera_input(delta);
            if is_key_pressed(KeyCode::G) {
                self.guide = (self.guide + 1) % 3;
                self.say(
                    ["Aim guide: full", "Aim guide: short", "Aim guide: off"][self.guide as usize],
                );
            }
            if is_key_pressed(KeyCode::H) {
                self.pocket_hints = !self.pocket_hints;
                self.say(if self.pocket_hints {
                    "Pocket hints: on"
                } else {
                    "Pocket hints: off"
                });
            }
        }
        self.cue_follow = (self.cue_follow - dt).max(0.0);
        if self.rack_sound {
            self.rack_sound = false;
            sfx.with(|s| s.rack());
        }
        // Two knocks on the rail.
        let before = self.tap;
        self.tap += dt;
        for at in [0.12f32, 0.37] {
            if before < at && self.tap >= at {
                sfx.with(|s| s.knock());
            }
        }

        match self.phase {
            Phase::Aim if !frozen_input => {
                if self.spin_drag {
                    let v = (m - lay.spin_c) / (lay.spin_r * 0.80);
                    let v = if v.length() > 1.0 { v.normalize() } else { v };
                    self.spin = vec2(v.x, -v.y);
                    if !is_mouse_button_down(MouseButton::Left) {
                        self.spin_drag = false;
                    }
                } else {
                    let looking = self.view != ViewMode::Top
                        && (is_mouse_button_down(MouseButton::Right)
                            || is_mouse_button_down(MouseButton::Middle));
                    if moved && !looking && !in_spin && m.y > lay.top {
                        if self.view == ViewMode::Cue {
                            // Relative aiming avoids a feedback loop when the
                            // camera itself turns to follow the cue.
                            self.aim -= delta.x as f64 * 0.0032;
                        } else {
                            let w = self.cam.to_world(m);
                            if (w - cue).len() > 0.012 {
                                self.aim = (w - cue).angle();
                            }
                        }
                    }
                    let fine = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
                    let rate = if fine { 0.03 } else { 0.35 } * dt as f64;
                    if is_key_down(KeyCode::Left) || is_key_down(KeyCode::A) {
                        self.aim += rate;
                    }
                    if is_key_down(KeyCode::Right) || is_key_down(KeyCode::D) {
                        self.aim -= rate;
                    }
                    let wheel = mouse_wheel().1;
                    if wheel != 0.0 && self.view == ViewMode::Top {
                        self.aim += wheel.signum() as f64 * 0.0015;
                    }
                    if is_key_pressed(KeyCode::Space) {
                        self.yaw = if (self.yaw + 0.72).abs() < 0.08 {
                            0.0
                        } else {
                            -0.72
                        };
                        self.say(if self.yaw == 0.0 {
                            "Shooter-side view"
                        } else {
                            "Centered cue view"
                        });
                    }
                    if is_key_pressed(KeyCode::C)
                        || (in_spin && is_mouse_button_pressed(MouseButton::Right))
                    {
                        self.spin = Vec2::ZERO;
                    }
                    let calling = self.rules.mode == Mode::EightBall && !self.rules.break_shot;
                    if is_key_pressed(KeyCode::S) && calling {
                        self.call.safety = !self.call.safety;
                    }
                    if is_key_pressed(KeyCode::P) && self.ctx.can_push {
                        self.call.push_out = !self.call.push_out;
                    }
                    let w = self.cam.to_world(m);
                    if is_mouse_button_pressed(MouseButton::Right) && calling && !in_spin {
                        let hit = self
                            .world
                            .balls
                            .iter()
                            .find(|b| b.id != 0 && b.on_table() && (b.pos - w).len() < R * 1.3)
                            .map(|b| b.id);
                        if let Some(id) = hit {
                            self.manual_ball = if self.manual_ball == Some(id) {
                                None
                            } else {
                                Some(id)
                            };
                        }
                    }
                    if is_mouse_button_pressed(MouseButton::Left) {
                        let pocket = self
                            .world
                            .pockets
                            .iter()
                            .position(|p| (p.center - w).len() < 0.085);
                        if in_spin {
                            self.spin_drag = true;
                        } else if self.rules.ball_in_hand && (m - cs).length() < ball_px * 1.7 {
                            self.phase = Phase::DragCue;
                        } else if let (true, Some(p)) = (calling, pocket) {
                            self.manual_pocket = if self.manual_pocket == Some(p) {
                                None
                            } else {
                                Some(p)
                            };
                        } else if m.y > lay.top {
                            self.power = 0.0;
                            self.phase = Phase::Pull { start: m, held: 0.0 };
                        }
                    }
                }
                self.update_call();
            }
            Phase::Pull { start, held } => {
                let held = held + dt;
                self.phase = Phase::Pull { start, held };
                // With the aim guide off the player judges the shot by where
                // the cursor rests, so it must not have to move: holding the
                // button draws the cue back by itself. Power climbs to full
                // in CHARGE seconds, then eases down and up again so an
                // overshoot can be waited out. With a guide showing, the
                // shot is drawn by dragging the mouse back along the cue.
                let ok;
                if self.guide == 2 {
                    const CHARGE: f32 = 1.5;
                    let x = (held / CHARGE) % 2.0;
                    let x = if x > 1.0 { 2.0 - x } else { x };
                    self.power = x.powf(1.4).clamp(0.0, 1.0);
                    // A quick click is not a shot.
                    ok = held > 0.14 && self.power > 0.004;
                } else {
                    let ahead = self.cam.to_screen(cue + V2::from_angle(self.aim) * 0.2);
                    let dir = (ahead - cs).normalize_or_zero();
                    let pull = (m - start).dot(-dir);
                    self.power = (pull / (190.0 * lay.ui)).clamp(0.0, 1.0);
                    ok = self.power > 0.03;
                }
                self.gap = self.power as f64 * 0.30;
                if is_mouse_button_pressed(MouseButton::Right)
                    || is_key_pressed(KeyCode::Escape)
                    || frozen_input
                {
                    self.power = 0.0;
                    self.gap = 0.0;
                    self.phase = Phase::Aim;
                } else if !is_mouse_button_down(MouseButton::Left) {
                    if ok {
                        self.phase = Phase::Stroke {
                            t: 0.0,
                            from: self.gap,
                            speed: power_to_speed(self.power),
                        };
                    } else {
                        self.power = 0.0;
                        self.gap = 0.0;
                        self.phase = Phase::Aim;
                    }
                }
            }
            Phase::DragCue => {
                self.try_place(self.cam.to_world(m));
                if !is_mouse_button_down(MouseButton::Left) || frozen_input {
                    self.phase = Phase::Aim;
                }
            }
            Phase::Stroke { t, from, speed } => {
                let t = t + dt;
                // Firm shots have a brisk stroke; soft shots take a touch
                // longer, while both remain responsive at normal frame rates.
                let duration = 0.125 - 0.040 * (from / 0.30).clamp(0.0, 1.0) as f32;
                let k = (t / duration).min(1.0);
                self.gap = from * (1.0 - (k * k) as f64);
                if k >= 1.0 {
                    self.fire(speed, sfx);
                } else {
                    self.phase = Phase::Stroke { t, from, speed };
                }
            }
            Phase::Rolling => {
                // Real time, always: the simulation consumes exactly the time
                // that passed, in fixed steps, whatever the frame rate is.
                self.acc += (dt as f64).min(0.25);
                while self.acc >= DT {
                    self.world.step();
                    self.acc -= DT;
                }
                let (mut ball, mut rail, mut pots) = (0.0f64, 0.0f64, 0);
                for e in self.world.events.drain(..) {
                    match e {
                        Event::BallHit(v) => ball = ball.max(v),
                        Event::CushionHit(v) => rail = rail.max(v),
                        Event::Pocketed(_) => pots += 1,
                    }
                }
                sfx.with(|s| {
                    if ball > 0.0 {
                        s.ball_hit(ball);
                    }
                    if rail > 0.0 {
                        s.cushion_hit(rail);
                    }
                    if pots > 0 {
                        s.pocket();
                    }
                });
                if self.world.at_rest() {
                    self.acc = 0.0;
                    if self.net.is_some() && self.shooter_local {
                        let state = Msg::State(snapshot(&self.world), self.world.log.clone());
                        self.send(state);
                        self.finish_shot(sfx);
                    } else if self.net.is_some() {
                        self.phase = Phase::WaitState { t: 0.0 };
                    } else {
                        self.finish_shot(sfx);
                    }
                }
            }
            Phase::WaitState { t } => {
                // Adopt the shooter's result so both tables stay identical.
                if let Some((balls, log)) = self.remote_state.take() {
                    for (id, on, x, y) in balls {
                        if let Some(b) = self.world.ball_mut(id) {
                            b.pos = physics::v2(x, y);
                            b.vel = V2::ZERO;
                            b.w = [0.0; 3];
                            b.state = if on {
                                BallState::OnTable
                            } else {
                                BallState::Pocketed
                            };
                        }
                    }
                    self.world.log = log;
                    self.finish_shot(sfx);
                } else if self.net.is_none() {
                    self.finish_shot(sfx);
                } else {
                    self.phase = Phase::WaitState { t: t + dt };
                }
            }
            Phase::AiThink { t } => {
                let t = t + dt;
                self.phase = Phase::AiThink { t };
                if t > 0.7 {
                    let got = match self.rx.as_ref().map(|rx| rx.try_recv()) {
                        Some(Ok(shot)) => Some(shot),
                        Some(Err(TryRecvError::Empty)) => None,
                        // The planner died; play a plain shot instead of hanging.
                        _ => Some(AiShot {
                            place: None,
                            angle: self.aim,
                            speed: 2.5,
                            side: 0.0,
                            top: 0.0,
                            call: ShotCall {
                                safety: true,
                                ..ShotCall::default()
                            },
                        }),
                    };
                    if let Some(shot) = got {
                        self.rx = None;
                        self.call = shot.call;
                        self.phase = Phase::AiMove {
                            shot,
                            t: 0.0,
                            from_angle: self.aim,
                            from_pos: cue,
                        };
                    }
                }
            }
            Phase::AiMove {
                shot,
                t,
                from_angle,
                from_pos,
            } => {
                let t = t + dt;
                if let Some(p) = shot.place {
                    let k = smooth(t / 0.4) as f64;
                    self.world.balls[0].pos = if k >= 1.0 {
                        p
                    } else {
                        from_pos + (p - from_pos) * k
                    };
                }
                let k = smooth((t - 0.4) / 0.65) as f64;
                self.aim = from_angle + angle_diff(from_angle, shot.angle) * k;
                let power = speed_to_power(shot.speed);
                let pull = smooth((t - 1.1) / 0.45);
                self.power = power * pull;
                self.gap = (power * pull) as f64 * 0.30;
                if t >= 1.62 {
                    self.aim = shot.angle;
                    self.spin = vec2(shot.side as f32, shot.top as f32);
                    self.phase = Phase::Stroke {
                        t: 0.0,
                        from: self.gap,
                        speed: shot.speed,
                    };
                } else {
                    self.phase = Phase::AiMove {
                        shot,
                        t,
                        from_angle,
                        from_pos,
                    };
                }
            }
            _ => {}
        }

        // Let the other computer watch the local player line up the shot.
        let local_aiming = matches!(
            self.phase,
            Phase::Aim | Phase::Pull { .. } | Phase::DragCue | Phase::AiMove { .. }
        );
        let every = if self.net.as_ref().is_some_and(|p| p.relayed()) { 0.1 } else { 0.05 };
        if self.net.is_some() && local_aiming && get_time() - self.cue_sent > every {
            self.cue_sent = get_time();
            let c = self.world.cue().pos;
            let msg = Msg::Cue {
                x: c.x,
                y: c.y,
                aim: self.aim,
                power: self.power as f64,
                side: self.spin.x as f64,
                top: self.spin.y as f64,
            };
            self.send(msg);
        }
        if matches!(self.phase, Phase::RemoteAim) {
            if let Some((aim, power)) = self.remote_cue {
                let k = 1.0 - (-dt as f64 * 14.0).exp();
                self.aim += angle_diff(self.aim, aim) * k;
                self.power += (power - self.power) * k as f32;
                self.gap = self.power as f64 * 0.30;
            }
        } else {
            self.remote_cue = None;
        }
        self.update_camera(dt, lay);
    }

    /// Recent chat lines and the chat box, bottom left.
    fn draw_chat(&self, r: &Renderer, lay: &Layout) {
        if self.net.is_none() && self.chat.is_empty() {
            return;
        }
        let ui = lay.ui;
        let now = get_time();
        let size = 14.0 * ui;
        let row = 24.0 * ui;
        let x = 16.0 * ui;
        let mut y = lay.sh - 64.0 * ui;
        if let Some(text) = &self.typing {
            let rect = Rect::new(x - 6.0 * ui, y - 14.0 * ui, 460.0 * ui, 28.0 * ui);
            r.panel(rect, 8.0 * ui, Color::new(0.0, 0.0, 0.0, 0.78), GOLD, 1.4);
            let cursor = if (now * 2.0) as i64 % 2 == 0 { "|" } else { "" };
            r.text(&format!("{text}{cursor}"), x + 4.0 * ui, y, size, INK, false);
            let left = net::CHAT_MAX - text.chars().count();
            r.text(
                &format!("Enter: send   Esc: close   {left} left"),
                rect.x + rect.w + 10.0 * ui,
                y,
                12.0 * ui,
                Color::new(1.0, 1.0, 1.0, 0.5),
                false,
            );
            y -= row + 8.0 * ui;
        }
        // Lines fade out after a while; opening the box brings them back.
        let open = self.typing.is_some();
        for (mine, text, at) in self.chat.iter().rev().take(if open { 8 } else { 5 }) {
            let age = (now - at) as f32;
            let alpha = if open { 1.0 } else { (12.0 - age).clamp(0.0, 1.0) };
            if alpha <= 0.0 {
                continue;
            }
            let who = if *mine { "You" } else { self.rules.names[1 - self.local].as_str() };
            let label = format!("{who}:  ");
            let lw = r.text_width(&label, size, true);
            let tw = r.text_width(text, size, false);
            let rect = Rect::new(x - 6.0 * ui, y - 11.0 * ui, lw + tw + 18.0 * ui, 22.0 * ui);
            r.panel(rect, 8.0 * ui, Color::new(0.0, 0.0, 0.0, 0.62 * alpha), Color::new(1.0, 1.0, 1.0, 0.10 * alpha), 1.0);
            let name_color = if *mine { GOLD } else { Color::new(0.55, 0.80, 1.0, 1.0) };
            r.text(&label, x, y, size, Color::new(name_color.r, name_color.g, name_color.b, alpha), true);
            r.text(text, x + lw, y, size, Color::new(INK.r, INK.g, INK.b, alpha), false);
            y -= row;
        }
    }

    /// The online shot clock, under the status line.
    fn draw_clock(&self, r: &Renderer, lay: &Layout) {
        let mine = matches!(self.phase, Phase::Aim | Phase::Pull { .. } | Phase::DragCue);
        if self.net.is_none() || !(mine || matches!(self.phase, Phase::RemoteAim)) {
            return;
        }
        let ui = lay.ui;
        let mid = lay.sw / 2.0;
        let y = if self.rules.mode == Mode::NineBall { 92.0 } else { 70.0 } * ui;
        let secs = self.clock.ceil() as i32;
        let low = self.clock <= 10.0;
        let color = if low {
            let pulse = 0.75 + 0.25 * (get_time() * 6.0).sin() as f32;
            Color::new(1.0, 0.32, 0.26, pulse)
        } else if mine {
            INK
        } else {
            DIM
        };
        let w = 150.0 * ui;
        let full = if self.clock > SHOT_CLOCK { BREAK_CLOCK } else { SHOT_CLOCK };
        let bar = Rect::new(mid - w / 2.0, y + 11.0 * ui, w, 4.0 * ui);
        r.panel(bar, 2.0 * ui, Color::new(1.0, 1.0, 1.0, 0.12), Color::new(0.0, 0.0, 0.0, 0.0), 0.0);
        let k = (self.clock / full).clamp(0.0, 1.0);
        if k > 0.0 {
            let fill = Rect::new(bar.x, bar.y, (bar.w * k).max(bar.h), bar.h);
            r.panel(fill, 2.0 * ui, if low { color } else { GOLD }, Color::new(0.0, 0.0, 0.0, 0.0), 0.0);
        }
        r.text_center(&format!("{}:{:02}", secs / 60, secs % 60), mid, y, 17.0 * ui, color, true);
        if mine && !self.extended[self.rules.current] {
            r.text("E: +30 s", mid + w / 2.0 + 10.0 * ui, y + 13.0 * ui, 11.5 * ui, Color::new(1.0, 1.0, 1.0, 0.5), false);
        }
    }

    fn showing_cue(&self) -> bool {
        matches!(
            self.phase,
            Phase::Aim
                | Phase::Pull { .. }
                | Phase::Stroke { .. }
                | Phase::AiMove { .. }
                | Phase::DragCue
                | Phase::RemoteAim
        )
    }

    fn ring(&self, p: V2, radius: f64, w: f32, c: Color) {
        let s = self.cam.to_screen(p);
        let px = self.cam.px_per_m(at(p, R as f32)) * radius as f32;
        draw_poly_lines(s.x, s.y, 48, px, 0.0, w, c);
    }

    fn line(&self, a: V2, b: V2, w: f32, c: Color) {
        let (p, q) = (self.cam.to_screen(a), self.cam.to_screen(b));
        draw_line(p.x, p.y, q.x, q.y, w, c);
    }

    fn draw_guides(&self) {
        let cue = self.world.cue().pos;
        let d = V2::from_angle(self.aim);
        let hit = cast_cue(&self.world, cue, d);
        let white = Color::new(1.0, 1.0, 1.0, 0.62);
        if self.guide == 2 {
            return;
        }
        if self.guide == 1 {
            // Short guide: a stub of the aiming line and nothing else.
            self.line(cue + d * R, cue + d * (R + hit.dist.min(0.35)), 1.6, white);
            return;
        }
        if hit.dist > R {
            self.line(cue + d * R, hit.point, 1.6, white);
        }
        match hit.ball.and_then(|id| self.world.ball(id)) {
            Some(target) => {
                let n = (target.pos - hit.point).norm();
                let cos = d.dot(n).clamp(0.0, 1.0);
                // Where the object ball goes, and where the cue ball glances off.
                self.line(
                    target.pos + n * R,
                    target.pos + n * (R + 0.10 + 0.42 * cos),
                    1.6,
                    Color::new(1.0, 0.86, 0.45, 0.80),
                );
                let tangent = d - n * d.dot(n);
                if tangent.len() > 0.05 {
                    self.line(
                        hit.point,
                        hit.point + tangent.norm() * (0.06 + 0.26 * tangent.len()),
                        1.3,
                        Color::new(1.0, 1.0, 1.0, 0.36),
                    );
                }
                let legal = self.ctx.legal.contains(&target.id)
                    || self.rules.mode == Mode::Practice
                    || self.call.push_out;
                self.ring(
                    hit.point,
                    R,
                    1.5,
                    if legal {
                        white
                    } else {
                        Color::new(1.0, 0.32, 0.28, 0.85)
                    },
                );
            }
            None => {
                let mut refl = d;
                if hit.point.x.abs() >= HALF_L - R - 1e-6 {
                    refl.x = -refl.x;
                } else {
                    refl.y = -refl.y;
                }
                self.line(
                    hit.point,
                    hit.point + refl * 0.22,
                    1.3,
                    Color::new(1.0, 1.0, 1.0, 0.30),
                );
                self.ring(hit.point, R, 1.3, Color::new(1.0, 1.0, 1.0, 0.40));
            }
        }
    }

    fn status(&self) -> String {
        let name = &self.rules.names[self.rules.current];
        let you = name == "You";
        match self.phase {
            Phase::AiThink { .. } => "Computer is thinking...".into(),
            Phase::AiMove { .. } => "Computer is lining up".into(),
            Phase::RemoteAim => "Opponent is lining up".into(),
            Phase::WaitState { .. } => "Syncing...".into(),
            Phase::Decide => "Waiting for a decision".into(),
            Phase::Rolling | Phase::Stroke { .. } | Phase::Over => String::new(),
            _ if self.rules.mode == Mode::Practice => {
                if self.rules.ball_in_hand {
                    "Drag the cue ball to place it".into()
                } else {
                    String::new()
                }
            }
            _ if self.rules.ball_in_hand => {
                let area = if self.rules.kitchen {
                    " behind the line"
                } else {
                    ""
                };
                if you {
                    format!("Ball in hand - drag the cue ball{area}")
                } else {
                    format!("{name}: ball in hand - drag the cue ball{area}")
                }
            }
            _ if you => "Your shot".into(),
            _ => format!("{name} to shoot"),
        }
    }

    fn draw_player(&self, r: &Renderer, lay: &Layout, p: usize, wins: [u32; 2]) {
        let ui = lay.ui;
        let w = 330.0 * ui;
        let rect = Rect::new(
            if p == 0 {
                12.0 * ui
            } else {
                lay.sw - 12.0 * ui - w
            },
            9.0 * ui,
            w,
            68.0 * ui,
        );
        let active = self.rules.current == p && !matches!(self.phase, Phase::Over);
        r.panel(
            rect,
            12.0 * ui,
            PANEL,
            if active { GOLD } else { EDGE },
            if active { 2.0 } else { 1.2 },
        );
        r.text(
            &self.rules.names[p],
            rect.x + 14.0 * ui,
            rect.y + 19.0 * ui,
            19.0 * ui,
            if active { INK } else { DIM },
            true,
        );
        r.text_right(
            &format!("{} won", wins[p]),
            rect.x + rect.w - 14.0 * ui,
            rect.y + 19.0 * ui,
            14.0 * ui,
            DIM,
            false,
        );
        let y = rect.y + 47.0 * ui;
        let br = 9.5 * ui;
        match self.rules.mode {
            Mode::EightBall => match self.rules.groups[p] {
                Some(g) => {
                    let mut x = rect.x + 14.0 * ui + br;
                    let left = Rules::remaining(&self.world, g);
                    for id in g.balls() {
                        r.draw_icon_ball(
                            vec2(x, y),
                            br,
                            id,
                            if self.world.on_table(id) { 1.0 } else { 0.16 },
                        );
                        x += br * 2.25;
                    }
                    x += br * 0.6;
                    r.draw_icon_ball(vec2(x, y), br, 8, if left == 0 { 1.0 } else { 0.22 });
                    let label = match g {
                        Group::Solids => "Solids",
                        Group::Stripes => "Stripes",
                    };
                    r.text_right(label, rect.x + rect.w - 14.0 * ui, y, 14.0 * ui, DIM, false);
                }
                None => r.text("Open table", rect.x + 14.0 * ui, y, 15.0 * ui, DIM, false),
            },
            Mode::NineBall => {
                let txt = if active {
                    Rules::lowest_ball(&self.world)
                        .map_or(String::new(), |n| format!("Must hit the {n} first"))
                } else {
                    "Waiting".to_string()
                };
                r.text(&txt, rect.x + 14.0 * ui, y, 15.0 * ui, DIM, false);
                let f = self.rules.fouls[p];
                if f > 0 {
                    let c = if f >= 2 {
                        Color::new(1.0, 0.4, 0.35, 1.0)
                    } else {
                        DIM
                    };
                    r.text_right(
                        &format!("Fouls in a row: {f}"),
                        rect.x + rect.w - 14.0 * ui,
                        y,
                        14.0 * ui,
                        c,
                        f >= 2,
                    );
                }
            }
            Mode::Practice => {}
        }
    }

    fn draw(&self, r: &mut Renderer, lay: &Layout, wins: [u32; 2]) {
        let ui = lay.ui;
        let lead = if matches!(self.phase, Phase::Rolling) {
            self.acc
        } else {
            0.0
        };
        r.draw_scene(&self.cam, &self.world, lead);
        let human_aiming = matches!(self.phase, Phase::Aim | Phase::Pull { .. } | Phase::DragCue);
        if self.rules.ball_in_hand
            && self.rules.kitchen
            && (human_aiming || matches!(self.phase, Phase::RemoteAim))
        {
            self.line(
                physics::v2(HEAD_SPOT.x, HALF_W),
                physics::v2(HEAD_SPOT.x, -HALF_W),
                1.5,
                Color::new(1.0, 1.0, 1.0, 0.30),
            );
        }
        let guides = matches!(
            self.phase,
            Phase::Aim | Phase::Pull { .. } | Phase::Stroke { .. } | Phase::RemoteAim
        ) || matches!(self.phase, Phase::AiMove { t, .. } if t > 0.4);
        if guides {
            self.draw_guides();
            if self.guide != 2 {
                if self.pocket_hints {
                    if let Some(p) = self.call.pocket {
                        self.ring(self.world.pockets[p].center, 0.075, 2.5, GOLD);
                    }
                }
                if let Some(b) = self
                    .call
                    .ball
                    .and_then(|id| self.world.ball(id))
                    .filter(|b| b.on_table())
                {
                    self.ring(b.pos, R * 1.25, 2.0, GOLD);
                }
            }
        }
        let cue = self.world.cue().pos;
        if self.rules.ball_in_hand && human_aiming {
            let pulse = 1.0 + 0.12 * ((get_time() * 4.0).sin());
            self.ring(
                cue,
                R * 1.55 * pulse,
                2.0,
                Color::new(GOLD.r, GOLD.g, GOLD.b, 0.85),
            );
        }
        if self.showing_cue() && !matches!(self.phase, Phase::DragCue) {
            r.draw_cue(&self.cam, cue, self.aim, self.gap, 1.0);
        } else if matches!(self.phase, Phase::Rolling) && self.cue_follow > 0.0 {
            let k = self.cue_follow / 0.12;
            r.draw_cue(
                &self.cam,
                self.cue_follow_origin,
                self.aim,
                -0.035 * (1.0 - k) as f64,
                k,
            );
        }

        // Heads-up display.
        if self.rules.mode == Mode::Practice {
            let left = self
                .world
                .balls
                .iter()
                .filter(|b| b.id != 0 && b.on_table())
                .count();
            let rect = Rect::new(12.0 * ui, 9.0 * ui, 330.0 * ui, 68.0 * ui);
            r.panel(rect, 12.0 * ui, PANEL, EDGE, 1.2);
            r.text(
                "Practice",
                rect.x + 14.0 * ui,
                rect.y + 19.0 * ui,
                19.0 * ui,
                INK,
                true,
            );
            r.text(
                &format!("{left} balls on the table"),
                rect.x + 14.0 * ui,
                rect.y + 47.0 * ui,
                15.0 * ui,
                DIM,
                false,
            );
        } else {
            self.draw_player(r, lay, 0, wins);
            self.draw_player(r, lay, 1, wins);
        }
        let mid = lay.sw / 2.0;
        let title = match self.rules.mode {
            Mode::EightBall => "8-BALL",
            Mode::NineBall => "9-BALL",
            Mode::Practice => "PRACTICE",
        };
        let title_line = match &self.net {
            Some(p) => {
                let path = match p.path() {
                    "RELAY" => "ONLINE (RELAY)",
                    "DIRECT" => "ONLINE (DIRECT)",
                    other => other,
                };
                match p.rtt_ms {
                    Some(ms) => format!("{title}   |   {path}   {ms} ms"),
                    None => format!("{title}   |   {path}"),
                }
            }
            None => title.to_string(),
        };
        r.text_center(
            &title_line,
            mid,
            20.0 * ui,
            13.0 * ui,
            Color::new(GOLD.r, GOLD.g, GOLD.b, 0.9),
            true,
        );
        let st = self.status();
        if self.rules.mode == Mode::NineBall {
            let br = 9.0 * ui;
            let lowest = Rules::lowest_ball(&self.world);
            for id in 1..=9u8 {
                let x = mid + (id as f32 - 5.0) * br * 2.3;
                if Some(id) == lowest {
                    draw_poly_lines(x, 46.0 * ui, 30, br * 1.35, 0.0, 1.6, GOLD);
                }
                r.draw_icon_ball(
                    vec2(x, 46.0 * ui),
                    br,
                    id,
                    if self.world.on_table(id) { 1.0 } else { 0.16 },
                );
            }
            r.text_center(&st, mid, 71.0 * ui, 14.0 * ui, DIM, false);
        } else {
            r.text_center(&st, mid, 46.0 * ui, 17.0 * ui, INK, false);
        }
        if human_aiming {
            let ct = self.call_text();
            if !ct.is_empty() {
                let warn = ct.starts_with("No call");
                let w = r.text_width(&ct, 14.0 * ui, true) + 28.0 * ui;
                let rect = Rect::new(mid - w / 2.0, lay.sh - 84.0 * ui, w, 26.0 * ui);
                let edge = if warn {
                    Color::new(1.0, 0.45, 0.35, 0.8)
                } else {
                    EDGE
                };
                r.panel(
                    rect,
                    13.0 * ui,
                    Color::new(0.04, 0.045, 0.06, 0.80),
                    edge,
                    1.2,
                );
                r.text_center(
                    &ct,
                    mid,
                    rect.y + rect.h / 2.0,
                    14.0 * ui,
                    if warn {
                        Color::new(1.0, 0.75, 0.7, 1.0)
                    } else {
                        INK
                    },
                    true,
                );
            }
        }

        // Referee message.
        let age = (get_time() - self.toast_at) as f32;
        if age < 4.2 && !self.toast.is_empty() {
            let a = smooth(age / 0.18) * smooth((4.2 - age) / 0.6);
            let w = r.text_width(&self.toast, 18.0 * ui, true) + 44.0 * ui;
            let rect = Rect::new(mid - w / 2.0, lay.top + 14.0 * ui, w, 38.0 * ui);
            r.panel(
                rect,
                19.0 * ui,
                Color::new(0.04, 0.045, 0.06, 0.86 * a),
                Color::new(GOLD.r, GOLD.g, GOLD.b, 0.75 * a),
                1.3,
            );
            r.text_center(
                &self.toast,
                mid,
                rect.y + rect.h / 2.0,
                18.0 * ui,
                Color::new(1.0, 1.0, 1.0, a),
                true,
            );
        }

        // Bottom bar: hints, power, spin.
        let by = lay.sh - 25.0 * ui;
        if human_aiming {
            let shoot = if self.guide == 2 { "hold: draw back, release: shoot" } else { "pull back: shoot" };
            let mut hint = if self.view == ViewMode::Top {
                format!("Point to aim  |  {shoot}  |  A/D: fine aim  |  V: view  |  G: guides  |  H: pockets  |  Esc: menu")
            } else {
                format!("Move mouse: aim  |  {shoot}  |  right-drag: orbit  |  wheel: zoom  |  Space: cue center  |  V: view  |  G: guides  |  H: pockets  |  Esc: menu")
            };
            if self.net.is_some() {
                hint.push_str("  |  Enter: chat");
            }
            r.text(
                &hint,
                16.0 * ui,
                by + 6.0 * ui,
                13.0 * ui,
                Color::new(1.0, 1.0, 1.0, 0.50),
                false,
            );
        }
        if self.net.is_some() && !human_aiming && self.typing.is_none() {
            r.text("Enter: chat", 16.0 * ui, by + 6.0 * ui, 13.0 * ui, Color::new(1.0, 1.0, 1.0, 0.50), false);
        }
        self.draw_chat(r, lay);
        self.draw_clock(r, lay);
        if self.power > 0.0 && self.showing_cue() {
            let w = 300.0 * ui;
            let rect = Rect::new(mid - w / 2.0, by - 22.0 * ui, w, 14.0 * ui);
            r.panel(rect, 7.0 * ui, Color::new(0.0, 0.0, 0.0, 0.65), EDGE, 1.2);
            let p = self.power;
            let col = Color::new(
                (p * 2.0).min(1.0),
                ((1.0 - p) * 2.0).min(1.0) * 0.85 + 0.1,
                0.18,
                1.0,
            );
            let inner = Rect::new(
                rect.x + 2.0,
                rect.y + 2.0,
                ((rect.w - 4.0) * p).max(rect.h - 4.0),
                rect.h - 4.0,
            );
            r.panel(inner, 6.0 * ui, col, Color::new(0.0, 0.0, 0.0, 0.0), 0.0);
            r.text_center(
                &format!("{:.0}%", p * 100.0),
                mid,
                rect.y - 12.0 * ui,
                14.0 * ui,
                INK,
                true,
            );
        }
        if self.showing_cue() {
            let c = lay.spin_c;
            let rr = lay.spin_r;
            let frame = Rect::new(
                c.x - rr - 8.0 * ui,
                c.y - rr - 26.0 * ui,
                rr * 2.0 + 16.0 * ui,
                rr * 2.0 + 34.0 * ui,
            );
            r.panel(frame, 12.0 * ui, PANEL, EDGE, 1.2);
            r.text_center("SPIN", c.x, c.y - rr - 12.0 * ui, 11.0 * ui, DIM, true);
            draw_poly(c.x, c.y, 48, rr, 0.0, Color::new(0.95, 0.94, 0.89, 1.0));
            draw_poly_lines(c.x, c.y, 48, rr, 0.0, 1.5, Color::new(0.0, 0.0, 0.0, 0.5));
            draw_line(
                c.x - rr,
                c.y,
                c.x + rr,
                c.y,
                1.0,
                Color::new(0.0, 0.0, 0.0, 0.10),
            );
            draw_line(
                c.x,
                c.y - rr,
                c.x,
                c.y + rr,
                1.0,
                Color::new(0.0, 0.0, 0.0, 0.10),
            );
            let t = c + vec2(self.spin.x, -self.spin.y) * rr * 0.80;
            draw_poly(
                t.x,
                t.y,
                24,
                rr * 0.19,
                0.0,
                Color::new(0.16, 0.40, 0.85, 1.0),
            );
            draw_poly_lines(
                t.x,
                t.y,
                24,
                rr * 0.19,
                0.0,
                1.2,
                Color::new(0.0, 0.0, 0.0, 0.6),
            );
        }
    }

    /// Decision, pause and game-over dialogs. Returns what the player chose.
    fn overlay(&mut self, r: &Renderer, lay: &Layout, sfx: &Sfx) -> Action {
        let ui = lay.ui;
        let mid = lay.sw / 2.0;
        let over = matches!(self.phase, Phase::Over) || self.disconnected;
        if matches!(self.phase, Phase::Decide) && !self.paused && !over {
            let Some(d) = self.rules.pending.clone() else {
                return Action::None;
            };
            if self.ctl_of(d.by) != Ctl::Human {
                return Action::None;
            }
            let (w, bh) = (470.0 * ui, 44.0 * ui);
            let h = 110.0 * ui + d.options.len() as f32 * (bh + 10.0 * ui);
            let rect = Rect::new(mid - w / 2.0, lay.sh / 2.0 - h / 2.0, w, h);
            r.panel(
                rect,
                18.0 * ui,
                Color::new(0.06, 0.068, 0.085, 0.97),
                Color::new(GOLD.r, GOLD.g, GOLD.b, 0.55),
                1.5,
            );
            r.text_center(
                &format!("{} - your choice", self.rules.names[d.by]),
                mid,
                rect.y + 34.0 * ui,
                21.0 * ui,
                GOLD,
                true,
            );
            r.text_center(&d.title, mid, rect.y + 66.0 * ui, 15.0 * ui, DIM, false);
            let mut y = rect.y + 94.0 * ui;
            let mut picked = None;
            for (i, c) in d.options.iter().enumerate() {
                let b = Rect::new(mid - (w - 56.0 * ui) / 2.0, y, w - 56.0 * ui, bh);
                if button(
                    r,
                    sfx,
                    b,
                    c.label(),
                    16.0 * ui,
                    if i == 0 { PRIMARY } else { PLAIN },
                ) {
                    picked = Some(*c);
                }
                y += bh + 10.0 * ui;
            }
            if let Some(c) = picked {
                self.decide(c, true);
            }
            return Action::None;
        }
        if !over && !self.paused {
            return Action::None;
        }
        draw_rectangle(0.0, 0.0, lay.sw, lay.sh, Color::new(0.0, 0.0, 0.0, 0.55));
        let (w, h) = (400.0 * ui, 330.0 * ui);
        let rect = Rect::new(mid - w / 2.0, lay.sh / 2.0 - h / 2.0, w, h);
        r.panel(
            rect,
            18.0 * ui,
            Color::new(0.06, 0.068, 0.085, 0.97),
            Color::new(GOLD.r, GOLD.g, GOLD.b, 0.55),
            1.5,
        );
        let online = self.opponent == Opponent::Online;
        if self.disconnected {
            r.text_center(
                "Opponent left",
                mid,
                rect.y + 56.0 * ui,
                28.0 * ui,
                INK,
                true,
            );
        } else if over {
            let name = self
                .rules
                .winner
                .map(|w| self.rules.names[w].clone())
                .unwrap_or_default();
            let line = if name == "You" {
                "You win".to_string()
            } else {
                format!("{name} wins")
            };
            r.text_center(&line, mid, rect.y + 48.0 * ui, 30.0 * ui, GOLD, true);
            r.text_center(&self.toast, mid, rect.y + 86.0 * ui, 15.0 * ui, DIM, false);
        } else {
            r.text_center(
                if online { "Menu" } else { "Paused" },
                mid,
                rect.y + 56.0 * ui,
                30.0 * ui,
                INK,
                true,
            );
        }
        let bw = w - 64.0 * ui;
        let bh = 44.0 * ui;
        let mut y = rect.y + 116.0 * ui;
        let mut action = Action::None;
        let b = |y: f32| Rect::new(mid - bw / 2.0, y, bw, bh);
        let can_restart = !online || (self.local == 0 && !self.disconnected);
        if over {
            if can_restart {
                if button(r, sfx, b(y), "Play again", 18.0 * ui, PRIMARY) {
                    action = Action::Restart;
                }
            } else if !self.disconnected {
                r.text_center(
                    "Waiting for the host to start the next game",
                    mid,
                    y + bh / 2.0,
                    14.0 * ui,
                    DIM,
                    false,
                );
            }
        } else if button(r, sfx, b(y), "Resume", 18.0 * ui, PRIMARY) {
            self.paused = false;
        }
        y += bh + 10.0 * ui;
        if !over && can_restart {
            if button(r, sfx, b(y), "Restart game", 17.0 * ui, PLAIN) {
                action = Action::Restart;
            }
            y += bh + 10.0 * ui;
        }
        if button(r, sfx, b(y), "Main menu", 17.0 * ui, PLAIN) {
            action = Action::Menu;
        }
        y += bh + 10.0 * ui;
        if button(r, sfx, b(y), "Quit to desktop", 17.0 * ui, PLAIN) {
            action = Action::Quit;
        }
        action
    }
}

fn settings_from(p: &prefs::Prefs, settings: &mut Settings) {
    settings.mode = match p.mode.as_str() {
        "9" => Mode::NineBall,
        "practice" => Mode::Practice,
        _ => Mode::EightBall,
    };
    settings.level = match p.level.as_str() {
        "easy" => Difficulty::Easy,
        "hard" => Difficulty::Hard,
        "elite" => Difficulty::Elite,
        _ => Difficulty::Medium,
    };
    settings.opponent = match p.opponent.as_str() {
        "human" => Opponent::Human,
        _ => Opponent::Ai(settings.level),
    };
    settings.view = match p.view.as_str() {
        "orbit" => ViewMode::Free,
        "top" => ViewMode::Top,
        _ => ViewMode::Cue,
    };
    settings.guide = p.guide.min(2);
    settings.pocket_hints = p.pocket_hints;
    settings.fullscreen = p.fullscreen;
}

fn prefs_from(settings: &Settings, theme: usize, sfx: &Sfx) -> prefs::Prefs {
    let sound = sfx.0.as_ref().is_none_or(|s| s.enabled);
    let music = sfx.0.as_ref().map_or(2, |s| s.music_level);
    let level = |d: Difficulty| match d {
        Difficulty::Easy => "easy",
        Difficulty::Medium => "medium",
        Difficulty::Hard => "hard",
        Difficulty::Elite => "elite",
    };
    prefs::Prefs {
        mode: match settings.mode {
            Mode::EightBall => "8",
            Mode::NineBall => "9",
            Mode::Practice => "practice",
        }
        .into(),
        // An online match is never restored on launch.
        opponent: match settings.opponent {
            Opponent::Human => "human",
            _ => "computer",
        }
        .into(),
        level: level(settings.level).into(),
        theme,
        view: match settings.view {
            ViewMode::Cue => "shooter",
            ViewMode::Free => "orbit",
            ViewMode::Top => "top",
        }
        .into(),
        guide: settings.guide,
        pocket_hints: settings.pocket_hints,
        sound,
        music,
        fullscreen: settings.fullscreen,
    }
}

fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

#[macroquad::main(conf)]
async fn main() {
    let capture = arg_value("--shot");
    // Capture on the clock instead of a frame count (--seconds N): the game
    // then runs in real time and stays open a few seconds after the shot,
    // so two online windows can each capture while both are still there.
    let capture_secs: Option<f64> = arg_value("--seconds").and_then(|s| s.parse().ok());
    let mut captured = false;
    let capture_at: u32 = arg_value("--frames")
        .and_then(|f| f.parse().ok())
        .unwrap_or(0);
    let scene = arg_value("--scene").unwrap_or_else(|| "menu".into());
    let mut r = Renderer::new();
    if let Some(t) = arg_value("--theme").and_then(|t| t.parse::<usize>().ok()) {
        r.theme = t % THEMES.len();
    }
    let mut sfx = Sfx(Sounds::load().await);
    let mut settings = Settings {
        mode: match arg_value("--mode").as_deref() {
            Some("9") => Mode::NineBall,
            Some("practice") => Mode::Practice,
            _ => Mode::EightBall,
        },
        opponent: Opponent::Ai(Difficulty::Medium),
        level: Difficulty::Medium,
        view: ViewMode::Cue,
        fullscreen: !windowed_start(),
        guide: match arg_value("--guide").as_deref() {
            Some("short") => 1,
            Some("off") => 2,
            _ => 0,
        },
        pocket_hints: !has_arg("--no-pocket-hints"),
        autoplay: has_arg("--autoplay"),
        log: arg_value("--log"),
    };
    // Automated runs (screenshots, self-play) never read or write the
    // player's saved settings.
    let scripted = capture.is_some() || settings.autoplay || has_arg("--host") || arg_value("--join").is_some();
    if !scripted {
        let saved = prefs::Prefs::load();
        settings_from(&saved, &mut settings);
        r.theme = saved.theme % THEMES.len();
        if let Some(s) = sfx.0.as_mut() {
            s.enabled = saved.sound;
            s.music_level = saved.music;
        }
        if !settings.fullscreen {
            set_fullscreen(false);
        }
    }
    let mut last_saved = prefs_from(&settings, r.theme, &sfx);
    // BPB_TEST_SOUND keeps sound on during self-play so it can be recorded.
    if (capture.is_some() || settings.autoplay) && std::env::var_os("BPB_TEST_SOUND").is_none() {
        if let Some(s) = sfx.0.as_mut() {
            s.muted = true;
        }
    }
    let code_file = arg_value("--code-file");
    match arg_value("--opponent").as_deref() {
        Some("human") => settings.opponent = Opponent::Human,
        Some("online") => settings.opponent = Opponent::Online,
        Some("easy") => settings.opponent = Opponent::Ai(Difficulty::Easy),
        Some("hard") => settings.opponent = Opponent::Ai(Difficulty::Hard),
        Some("elite") => settings.opponent = Opponent::Ai(Difficulty::Elite),
        _ => {}
    }
    let mut demo = World::new();
    rack(&mut demo, settings.mode, 9);
    let mut game: Option<Game> = None;
    let mut lobby = Lobby::Idle;
    let mut menu_state = MenuState::new();
    let mut wins = [0u32; 2];
    let mut breaker = 0usize;
    let mut frame = 0u32;

    // Command-line shortcuts, used for automated checks.
    if has_arg("--host") {
        settings.opponent = Opponent::Online;
        lobby = Lobby::Creating(net::create_room(net::Paths::all()));
    } else if let Some(code) = arg_value("--join") {
        settings.opponent = Opponent::Online;
        menu_state.code = code.clone();
        lobby = Lobby::Joining(net::join_room(&code, net::Paths::all()), code);
    } else if settings.autoplay || (capture.is_some() && scene != "menu" && scene != "icons") {
        if scene == "two" {
            settings.opponent = Opponent::Human;
        }
        let mut g = Game::new(&settings, 0, 12345, None, 0);
        // Capture-only camera override; normal matches always begin in the
        // shooter view and transition automatically as play progresses.
        g.view = match arg_value("--view").as_deref() {
            Some("top") => ViewMode::Top,
            Some("free") => ViewMode::Free,
            _ => ViewMode::Cue,
        };
        if !settings.autoplay {
            g.toast_at = -100.0;
            g.aim = arg_value("--aim")
                .and_then(|a| a.parse::<f64>().ok())
                .unwrap_or(0.035);
            g.yaw = 0.0;
            if let Some(k) = arg_value("--angle").and_then(|a| a.parse::<f32>().ok()) {
                g.fyaw = k;
                g.fpitch = 0.60;
                g.fdist = 3.3;
            }
            if scene == "play" {
                g.world.begin_shot();
                g.world.strike(0.012, 8.6, 0.0, 0.1);
                g.phase = Phase::Rolling;
            } else if scene == "stroke" {
                g.rules.ball_in_hand = false;
                g.gap = 0.30;
                g.power = 1.0;
                g.phase = Phase::Stroke {
                    t: 0.0,
                    from: 0.30,
                    speed: 6.5,
                };
                if g.view == ViewMode::Cue {
                    g.cam = g.player_cam();
                }
            }
        }
        game = Some(g);
    }

    loop {
        frame += 1;
        let lay = layout();
        if lay.sw < 64.0 || lay.sh < 64.0 {
            next_frame().await;
            continue;
        }
        let dt = if capture.is_some() && !settings.autoplay && capture_secs.is_none() {
            1.0 / 60.0
        } else {
            get_frame_time()
        };
        if is_key_pressed(KeyCode::F11) {
            settings.fullscreen = !settings.fullscreen;
            set_fullscreen(settings.fullscreen);
        }
        if let Some(s) = sfx.0.as_mut() {
            s.update_music(game.is_some(), dt).await;
        }
        clear_background(BLACK);

        // Online lobby: turn a finished connection into a game.
        let mut start: Option<(Peer, usize, u64, usize)> = None;
        lobby = match std::mem::replace(&mut lobby, Lobby::Idle) {
            Lobby::Creating(rx) => match rx.try_recv() {
                Ok(Ok(room)) => {
                    if let Some(path) = &code_file {
                        let _ = std::fs::write(path, &room.code);
                    }
                    Lobby::Hosting(room)
                }
                Ok(Err(e)) => Lobby::Failed(e),
                Err(TryRecvError::Empty) => Lobby::Creating(rx),
                Err(TryRecvError::Disconnected) => Lobby::Failed("Could not open a room".into()),
            },
            Lobby::Hosting(mut room) => match room.poll() {
                Some(mut peer) => {
                    let seed = fresh_seed();
                    peer.send(&Msg::Start {
                        mode: mode_code(settings.mode),
                        seed,
                        first: 0,
                    });
                    start = Some((peer, 0, seed, 0));
                    Lobby::Idle
                }
                None => Lobby::Hosting(room),
            },
            Lobby::Joining(rx, code) => match rx.try_recv() {
                Ok(Ok(peer)) => Lobby::Joined(peer),
                Ok(Err(e)) => Lobby::Failed(e),
                Err(TryRecvError::Empty) => Lobby::Joining(rx, code),
                Err(TryRecvError::Disconnected) => Lobby::Failed("Could not connect".into()),
            },
            Lobby::Joined(mut peer) => {
                let mut begun = None;
                for m in peer.poll() {
                    if let Msg::Start { mode, seed, first } = m {
                        begun = Some((mode, seed, first));
                    }
                }
                match begun {
                    Some((mode, seed, first)) => {
                        settings.mode = if mode == 1 {
                            Mode::NineBall
                        } else {
                            Mode::EightBall
                        };
                        start = Some((peer, 1, seed, (first as usize).min(1)));
                        Lobby::Idle
                    }
                    None if !peer.alive => Lobby::Failed("The host closed the connection".into()),
                    None => Lobby::Joined(peer),
                }
            }
            other => other,
        };
        if let Some((peer, local, seed, first)) = start {
            settings.opponent = Opponent::Online;
            wins = [0, 0];
            breaker = first;
            game = Some(Game::new(&settings, first, seed, Some(peer), local));
        }

        let mut quit = false;
        let mut next: Option<Game> = None;
        let mut to_menu = false;
        if let Some(g) = game.as_mut() {
            let over = matches!(g.phase, Phase::Over);
            let online = g.opponent == Opponent::Online;
            if is_key_pressed(KeyCode::Escape) && !over && !matches!(g.phase, Phase::Pull { .. }) && g.typing.is_none() {
                g.paused = !g.paused;
            }
            if !g.paused || online {
                let frozen = (capture.is_some() && !settings.autoplay) || g.paused;
                g.update(dt, &lay, &sfx, frozen);
            }
            // A guest starts the next game when the host says so.
            if let Some((seed, first)) = g.restart.take() {
                if online && g.local == 1 {
                    breaker = first;
                    let peer = g.net.take();
                    next = Some(Game::new(&settings, first, seed, peer, 1));
                }
            }
            g.draw(&mut r, &lay, wins);
            // Count the win once, when the game ends.
            if matches!(g.phase, Phase::Over) && !over {
                if let Some(w) = g.rules.winner {
                    wins[w] += 1;
                }
            }
            match g.overlay(&r, &lay, &sfx) {
                Action::None => {}
                Action::Restart => {
                    if matches!(g.phase, Phase::Over) {
                        breaker = 1 - breaker;
                    }
                    let seed = fresh_seed();
                    let mut peer = g.net.take();
                    if let Some(p) = peer.as_mut() {
                        p.send(&Msg::Start {
                            mode: mode_code(settings.mode),
                            seed,
                            first: breaker as u8,
                        });
                    }
                    let local = g.local;
                    next = Some(Game::new(&settings, breaker, seed, peer, local));
                }
                Action::Menu => to_menu = true,
                Action::Quit => quit = true,
            }
        } else {
            match menu::menu(
                &mut r,
                &lay,
                &mut sfx,
                &mut settings,
                &mut demo,
                &mut lobby,
                &mut menu_state,
            ) {
                MenuAction::None => {}
                MenuAction::Play => {
                    wins = [0, 0];
                    breaker = 0;
                    game = Some(Game::new(&settings, breaker, fresh_seed(), None, 0));
                }
                MenuAction::Quit => quit = true,
            }
        }
        if let Some(g) = next {
            game = Some(g);
        }
        if to_menu {
            if let Some(g) = game.as_ref() {
                settings.view = g.view;
                settings.guide = g.guide;
                settings.pocket_hints = g.pocket_hints;
            }
            game = None;
            lobby = Lobby::Idle;
            if settings.opponent == Opponent::Online {
                settings.opponent = Opponent::Ai(settings.level);
            }
            rack(&mut demo, settings.mode, frame as u64);
        }

        if capture.is_some() && scene == "icons" {
            draw_rectangle(0.0, 0.0, lay.sw, lay.sh, Color::new(0.1, 0.3, 0.2, 1.0));
            for id in 0..=15u8 {
                let at = vec2(
                    110.0 + (id % 8) as f32 * 150.0,
                    220.0 + (id / 8) as f32 * 220.0,
                );
                r.draw_icon_ball(at, 62.0, id, 1.0);
            }
        }
        if let Some(path) = &capture {
            let at = if capture_at > 0 {
                capture_at
            } else if scene == "play" {
                70
            } else {
                90
            };
            let due = match capture_secs {
                Some(secs) => {
                    quit |= get_time() >= secs + 6.0;
                    !captured && get_time() >= secs
                }
                None => frame == at,
            };
            if due {
                captured = true;
                let mut img = get_screen_data();
                for px in img.bytes.chunks_mut(4) {
                    px[3] = 255;
                }
                img.export_png(path);
                quit |= capture_secs.is_none();
            }
        }
        if !scripted {
            let now = prefs_from(&settings, r.theme, &sfx);
            if now != last_saved {
                now.save();
                last_saved = now;
            }
        }
        if quit {
            break;
        }
        next_frame().await;
    }
}
