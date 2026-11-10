//! Deterministic pool physics.
//!
//! Units are SI (metres, seconds, radians). The frame is right-handed with
//! x along the long axis of the table, y across it and z pointing up.
//! The simulation always advances in fixed steps of [`DT`], so the same shot
//! produces the same result every time, on any machine, at any frame rate.

use std::f64::consts::PI;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

/// Fixed simulation step (960 Hz).
pub const DT: f64 = 1.0 / 960.0;
/// Ball radius (2.25 inch ball).
pub const R: f64 = 0.028575;
/// Half length of the playing surface (cushion nose to nose), 9 ft table.
pub const HALF_L: f64 = 1.27;
/// Half width of the playing surface.
pub const HALF_W: f64 = 0.635;
/// Depth of the cushion from the nose back to the wooden rail.
pub const CUSHION_DEPTH: f64 = 0.05;
/// Distance from the table corner to where the cushion nose ends.
pub const CORNER_CUT: f64 = 0.0863;
/// Half width of the side pocket mouth.
pub const SIDE_CUT: f64 = 0.0675;
/// Radius of every pocket hole.
pub const POCKET_R: f64 = 0.07;
/// Radius of the side pocket holes.
pub const SIDE_POCKET_R: f64 = 0.062;

const G: f64 = 9.81;
/// Ball/cloth sliding friction.
const MU_SLIDE: f64 = 0.2;
/// Ball/cloth rolling resistance.
const MU_ROLL: f64 = 0.0125;
/// Decay of spin about the vertical axis (rad/s^2).
const SPIN_DECAY: f64 = 22.0;
/// Ball/ball restitution.
const E_BALL: f64 = 0.95;
/// Ball/ball friction (throw and spin transfer).
const MU_BALL: f64 = 0.06;
/// Cushion restitution at low speed.
const E_CUSHION: f64 = 0.85;
/// Ball/cushion friction (running and reverse english).
const MU_CUSHION: f64 = 0.2;
/// How much of the forward roll survives a cushion hit (negative = reversed).
const CUSHION_ROLL_KEEP: f64 = -0.35;
/// Seconds a ball takes to drop into a pocket.
pub const POCKET_TIME: f64 = 0.28;
/// Largest cue tip offset from the centre of the ball, as a fraction of R.
pub const MAX_TIP_OFFSET: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct V2 {
    pub x: f64,
    pub y: f64,
}

pub const fn v2(x: f64, y: f64) -> V2 {
    V2 { x, y }
}

impl V2 {
    pub const ZERO: V2 = V2 { x: 0.0, y: 0.0 };
    pub fn dot(self, o: V2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn len2(self) -> f64 {
        self.dot(self)
    }
    pub fn len(self) -> f64 {
        self.len2().sqrt()
    }
    pub fn norm(self) -> V2 {
        let l = self.len();
        if l > 1e-12 {
            self * (1.0 / l)
        } else {
            V2::ZERO
        }
    }
    /// Rotated 90 degrees counter-clockwise.
    pub fn perp(self) -> V2 {
        v2(-self.y, self.x)
    }
    pub fn from_angle(a: f64) -> V2 {
        v2(a.cos(), a.sin())
    }
    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        v2(self.x + o.x, self.y + o.y)
    }
}
impl Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        v2(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for V2 {
    type Output = V2;
    fn mul(self, k: f64) -> V2 {
        v2(self.x * k, self.y * k)
    }
}
impl Neg for V2 {
    type Output = V2;
    fn neg(self) -> V2 {
        v2(-self.x, -self.y)
    }
}
impl AddAssign for V2 {
    fn add_assign(&mut self, o: V2) {
        self.x += o.x;
        self.y += o.y;
    }
}
impl SubAssign for V2 {
    fn sub_assign(&mut self, o: V2) {
        self.x -= o.x;
        self.y -= o.y;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BallState {
    OnTable,
    /// Dropping into a pocket: pocket index and seconds elapsed.
    Pocketing(usize, f64),
    Pocketed,
}

#[derive(Clone, Debug)]
pub struct Ball {
    /// 0 is the cue ball, 1..=15 the object balls.
    pub id: u8,
    pub pos: V2,
    pub vel: V2,
    /// Angular velocity (x, y, z).
    pub w: [f64; 3],
    /// Orientation quaternion (w, x, y, z), object to world.
    pub q: [f64; 4],
    pub state: BallState,
}

impl Ball {
    pub fn new(id: u8, pos: V2) -> Ball {
        // A varied starting orientation so the rack does not look stamped out.
        let a = id as f64 * 2.399963;
        let axis = [a.cos() * 0.6, a.sin() * 0.6, 0.53];
        let mut b = Ball {
            id,
            pos,
            vel: V2::ZERO,
            w: [0.0; 3],
            q: [1.0, 0.0, 0.0, 0.0],
            state: BallState::OnTable,
        };
        if id != 0 {
            b.rotate(axis, 1.0);
        }
        b
    }

    pub fn on_table(&self) -> bool {
        self.state == BallState::OnTable
    }

    /// True while the ball still has motion that can change the layout.
    pub fn moving(&self) -> bool {
        match self.state {
            BallState::OnTable => {
                self.vel.x != 0.0 || self.vel.y != 0.0 || self.w[0] != 0.0 || self.w[1] != 0.0
            }
            BallState::Pocketing(..) => true,
            BallState::Pocketed => false,
        }
    }

    fn rotate(&mut self, w: [f64; 3], dt: f64) {
        let mag = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        let angle = mag * dt;
        if angle < 1e-12 {
            return;
        }
        let s = (angle * 0.5).sin() / mag;
        let d = [(angle * 0.5).cos(), w[0] * s, w[1] * s, w[2] * s];
        let q = self.q;
        let mut r = [
            d[0] * q[0] - d[1] * q[1] - d[2] * q[2] - d[3] * q[3],
            d[0] * q[1] + d[1] * q[0] + d[2] * q[3] - d[3] * q[2],
            d[0] * q[2] - d[1] * q[3] + d[2] * q[0] + d[3] * q[1],
            d[0] * q[3] + d[1] * q[2] - d[2] * q[1] + d[3] * q[0],
        ];
        let n = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3]).sqrt();
        if n > 1e-12 && n.is_finite() {
            for v in r.iter_mut() {
                *v /= n;
            }
            self.q = r;
        }
    }

    /// Columns of the object-to-world rotation matrix.
    pub fn rotation_columns(&self) -> [[f32; 3]; 3] {
        let [w, x, y, z] = self.q;
        [
            [
                (1.0 - 2.0 * (y * y + z * z)) as f32,
                (2.0 * (x * y + w * z)) as f32,
                (2.0 * (x * z - w * y)) as f32,
            ],
            [
                (2.0 * (x * y - w * z)) as f32,
                (1.0 - 2.0 * (x * x + z * z)) as f32,
                (2.0 * (y * z + w * x)) as f32,
            ],
            [
                (2.0 * (x * z + w * y)) as f32,
                (2.0 * (y * z - w * x)) as f32,
                (1.0 - 2.0 * (x * x + y * y)) as f32,
            ],
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub a: V2,
    pub b: V2,
}

#[derive(Clone, Copy, Debug)]
pub struct Pocket {
    pub center: V2,
    pub radius: f64,
    /// Point in the mouth of the pocket that a ball should be aimed at.
    pub aim: V2,
}

/// Something audible that happened during a step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    BallHit(f64),
    CushionHit(f64),
    Pocketed(u8),
}

/// What happened between the cue strike and the balls coming to rest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShotLog {
    /// First object ball the cue ball touched.
    pub first_hit: Option<u8>,
    /// Object balls pocketed, in order.
    pub potted: Vec<u8>,
    /// Pocket index for each entry of `potted`.
    pub pockets: Vec<usize>,
    /// The cue ball crossed the head string before its first contact.
    pub crossed_head_string: bool,
    /// The first ball contacted was behind the head string at that moment.
    pub first_hit_in_kitchen: bool,
    pub cue_potted: bool,
    /// A ball reached a cushion after the cue ball's first contact.
    pub cushion_after_contact: bool,
    /// Number of different object balls that touched a cushion (break rule).
    pub balls_to_cushion: u32,
    pub cushion_mask: u16,
}

#[derive(Clone, Debug)]
pub struct World {
    pub balls: Vec<Ball>,
    pub segs: Vec<Seg>,
    pub pockets: Vec<Pocket>,
    pub events: Vec<Event>,
    pub log: ShotLog,
}

fn build_table() -> (Vec<Seg>, Vec<Pocket>) {
    let mut segs = Vec::new();
    let (l, w) = (HALF_L, HALF_W);
    // Jaw facing directions for one quadrant (x > 0, y > 0).
    let a38 = 38.0_f64.to_radians();
    let a76 = 76.0_f64.to_radians();
    let corner_len = CUSHION_DEPTH / a38.sin();
    let side_len = CUSHION_DEPTH / a76.sin();
    for &sx in &[-1.0, 1.0] {
        for &sy in &[-1.0, 1.0] {
            let m = |p: V2| v2(p.x * sx, p.y * sy);
            // Long rail nose, from the side pocket to the corner pocket.
            let p1 = v2(SIDE_CUT, w);
            let p2 = v2(l - CORNER_CUT, w);
            segs.push(Seg { a: m(p1), b: m(p2) });
            // Side pocket jaw.
            let j1 = p1 + v2(-a76.cos(), a76.sin()) * side_len;
            segs.push(Seg { a: m(p1), b: m(j1) });
            // Corner pocket jaw on the long rail.
            let j2 = p2 + v2(a38.cos(), a38.sin()) * corner_len;
            segs.push(Seg { a: m(p2), b: m(j2) });
            // Short rail nose (half of it, mirrored by sy).
            let p3 = v2(l, w - CORNER_CUT);
            segs.push(Seg {
                a: m(v2(l, 0.0)),
                b: m(p3),
            });
            // Corner pocket jaw on the short rail.
            let j3 = p3 + v2(a38.sin(), a38.cos()) * corner_len;
            segs.push(Seg { a: m(p3), b: m(j3) });
        }
    }
    let mut pockets = Vec::new();
    for &sx in &[-1.0, 1.0] {
        for &sy in &[-1.0, 1.0] {
            pockets.push(Pocket {
                center: v2(sx * (l + 0.02), sy * (w + 0.02)),
                radius: POCKET_R,
                aim: v2(sx * (l - 0.025), sy * (w - 0.025)),
            });
        }
    }
    for &sy in &[-1.0, 1.0] {
        pockets.push(Pocket {
            center: v2(0.0, sy * (w + 0.04)),
            radius: SIDE_POCKET_R,
            aim: v2(0.0, sy * (w + 0.005)),
        });
    }
    (segs, pockets)
}

fn closest_on_seg(p: V2, s: &Seg) -> V2 {
    let ab = s.b - s.a;
    let t = ((p - s.a).dot(ab) / ab.len2()).clamp(0.0, 1.0);
    s.a + ab * t
}

impl World {
    pub fn new() -> World {
        let (segs, pockets) = build_table();
        World {
            balls: Vec::new(),
            segs,
            pockets,
            events: Vec::new(),
            log: ShotLog::default(),
        }
    }

    pub fn ball(&self, id: u8) -> Option<&Ball> {
        self.balls.iter().find(|b| b.id == id)
    }

    pub fn ball_mut(&mut self, id: u8) -> Option<&mut Ball> {
        self.balls.iter_mut().find(|b| b.id == id)
    }

    pub fn cue(&self) -> &Ball {
        &self.balls[0]
    }

    pub fn on_table(&self, id: u8) -> bool {
        self.ball(id).map_or(false, |b| b.on_table())
    }

    /// True when nothing on the table can move any more.
    pub fn at_rest(&self) -> bool {
        !self.balls.iter().any(|b| b.moving())
    }

    /// True if a ball centred at `p` would not overlap any ball except `ignore`.
    pub fn spot_free(&self, p: V2, ignore: u8) -> bool {
        self.balls
            .iter()
            .filter(|b| b.id != ignore && b.on_table())
            .all(|b| (b.pos - p).len2() >= (2.0 * R + 0.0005) * (2.0 * R + 0.0005))
    }

    /// True if `p` is a legal resting place for a ball centre.
    pub fn in_bounds(p: V2) -> bool {
        p.x.abs() <= HALF_L - R - 0.001 && p.y.abs() <= HALF_W - R - 0.001
    }

    /// Nearest free spot to `want`, searching outwards in rings.
    pub fn find_free_spot(&self, want: V2, ignore: u8) -> V2 {
        let clamp = |p: V2| {
            v2(
                p.x.clamp(-(HALF_L - R - 0.002), HALF_L - R - 0.002),
                p.y.clamp(-(HALF_W - R - 0.002), HALF_W - R - 0.002),
            )
        };
        let want = clamp(want);
        if self.spot_free(want, ignore) {
            return want;
        }
        for ring in 1..400 {
            let rad = ring as f64 * R * 0.5;
            let n = 8 + ring * 4;
            for k in 0..n {
                let a = k as f64 / n as f64 * 2.0 * PI;
                let p = clamp(want + V2::from_angle(a) * rad);
                if self.spot_free(p, ignore) {
                    return p;
                }
            }
        }
        want
    }

    /// Put a ball back on the table at rest, as close to `want` as possible.
    pub fn place_ball(&mut self, id: u8, want: V2) {
        let p = self.find_free_spot(want, id);
        if let Some(b) = self.ball_mut(id) {
            b.pos = p;
            b.vel = V2::ZERO;
            b.w = [0.0; 3];
            b.state = BallState::OnTable;
        }
    }

    /// Return a ball to the table the official way: on the long string, as
    /// close to the foot spot as possible, towards the foot rail first.
    pub fn spot_ball(&mut self, id: u8) {
        let step = 0.001;
        let mut found = None;
        let mut x = FOOT_SPOT.x;
        while x <= HALF_L - R - 0.001 {
            if self.spot_free(v2(x, 0.0), id) {
                found = Some(x);
                break;
            }
            x += step;
        }
        if found.is_none() {
            let mut x = FOOT_SPOT.x;
            while x >= -(HALF_L - R - 0.001) {
                if self.spot_free(v2(x, 0.0), id) {
                    found = Some(x);
                    break;
                }
                x -= step;
            }
        }
        let p = v2(found.unwrap_or(FOOT_SPOT.x), 0.0);
        if let Some(b) = self.ball_mut(id) {
            b.pos = p;
            b.vel = V2::ZERO;
            b.w = [0.0; 3];
            b.state = BallState::OnTable;
        }
    }

    /// Clear the shot log; call right before striking the cue ball.
    pub fn begin_shot(&mut self) {
        self.log = ShotLog::default();
        self.events.clear();
    }

    /// Strike the cue ball.
    ///
    /// `angle` is the aim direction, `speed` the resulting ball speed in m/s.
    /// `side` and `top` are the tip offsets from the centre of the ball in the
    /// range -1..=1 (right english and follow are positive).
    pub fn strike(&mut self, angle: f64, speed: f64, side: f64, top: f64) {
        let d = V2::from_angle(angle);
        let mut a = side.clamp(-1.0, 1.0) * MAX_TIP_OFFSET;
        let mut b = top.clamp(-1.0, 1.0) * MAX_TIP_OFFSET;
        let m = (a * a + b * b).sqrt();
        if m > MAX_TIP_OFFSET {
            a *= MAX_TIP_OFFSET / m;
            b *= MAX_TIP_OFFSET / m;
        }
        let left = d.perp();
        let k = 2.5 * speed / R;
        let cue = &mut self.balls[0];
        cue.vel = d * speed;
        cue.w = [left.x * k * b, left.y * k * b, k * a];
    }

    /// Advance the simulation by one fixed step.
    pub fn step(&mut self) {
        let dt = DT;
        for b in self.balls.iter_mut() {
            match b.state {
                BallState::OnTable => integrate(b, dt),
                BallState::Pocketing(p, t) => {
                    let t = t + dt;
                    let w = b.w;
                    b.rotate(w, dt);
                    let c = self.pockets[p].center;
                    b.pos = b.pos + (c - b.pos) * (dt / POCKET_TIME * 4.0).min(1.0);
                    b.state = if t >= POCKET_TIME {
                        b.vel = V2::ZERO;
                        b.w = [0.0; 3];
                        BallState::Pocketed
                    } else {
                        BallState::Pocketing(p, t)
                    };
                }
                BallState::Pocketed => {}
            }
        }
        if self.log.first_hit.is_none()
            && self.balls[0].on_table()
            && self.balls[0].pos.x > HEAD_SPOT.x
        {
            self.log.crossed_head_string = true;
        }
        self.collide_balls();
        self.collide_cushions();
        self.check_pockets();
    }

    fn collide_balls(&mut self) {
        let n = self.balls.len();
        let d2max = 4.0 * R * R;
        for i in 0..n {
            if !self.balls[i].on_table() {
                continue;
            }
            for j in (i + 1)..n {
                if !self.balls[j].on_table() {
                    continue;
                }
                if !self.balls[i].moving() && !self.balls[j].moving() {
                    continue;
                }
                let delta = self.balls[j].pos - self.balls[i].pos;
                if delta.x.abs() > 2.0 * R || delta.y.abs() > 2.0 * R {
                    continue;
                }
                let d2 = delta.len2();
                if d2 >= d2max {
                    continue;
                }
                let dist = d2.sqrt();
                let nrm = if dist > 1e-9 {
                    delta * (1.0 / dist)
                } else {
                    v2(1.0, 0.0)
                };
                // Separate the overlap.
                let push = (2.0 * R - dist) * 0.5;
                self.balls[i].pos -= nrm * push;
                self.balls[j].pos += nrm * push;

                let (va, wa) = (self.balls[i].vel, self.balls[i].w);
                let (vb, wb) = (self.balls[j].vel, self.balls[j].w);
                let vn = (va - vb).dot(nrm);
                if vn <= 0.0 {
                    continue;
                }
                let jn = 0.5 * (1.0 + E_BALL) * vn;

                // Relative surface velocity at the contact point, tangent part.
                // Contact velocity of a ball is v + w x r, with r = +-R n.
                let cross = |w: [f64; 3], s: f64| -> [f64; 3] {
                    // w x (s * R * n), n in the plane.
                    let (rx, ry) = (s * R * nrm.x, s * R * nrm.y);
                    [-w[2] * ry, w[2] * rx, w[0] * ry - w[1] * rx]
                };
                let ca = cross(wa, 1.0);
                let cb = cross(wb, -1.0);
                let rel = [
                    va.x + ca[0] - vb.x - cb[0],
                    va.y + ca[1] - vb.y - cb[1],
                    ca[2] - cb[2],
                ];
                let rn = rel[0] * nrm.x + rel[1] * nrm.y;
                let tan = [rel[0] - rn * nrm.x, rel[1] - rn * nrm.y, rel[2]];
                let tl = (tan[0] * tan[0] + tan[1] * tan[1] + tan[2] * tan[2]).sqrt();
                let mut jt = [0.0; 3];
                if tl > 1e-9 {
                    let mag = (MU_BALL * jn).min(tl / 7.0);
                    jt = [-tan[0] / tl * mag, -tan[1] / tl * mag, -tan[2] / tl * mag];
                }
                // n x jt, used for the spin change of both balls.
                let k = 2.5 / R;
                let nxj = [nrm.y * jt[2], -nrm.x * jt[2], nrm.x * jt[1] - nrm.y * jt[0]];

                let a = &mut self.balls[i];
                a.vel = a.vel - nrm * jn + v2(jt[0], jt[1]);
                a.w = [
                    a.w[0] + k * nxj[0],
                    a.w[1] + k * nxj[1],
                    a.w[2] + k * nxj[2],
                ];
                let ida = a.id;
                let b = &mut self.balls[j];
                b.vel = b.vel + nrm * jn - v2(jt[0], jt[1]);
                b.w = [
                    b.w[0] + k * nxj[0],
                    b.w[1] + k * nxj[1],
                    b.w[2] + k * nxj[2],
                ];
                let idb = b.id;

                if self.log.first_hit.is_none() && (ida == 0 || idb == 0) {
                    let (other, at) = if ida == 0 {
                        (idb, self.balls[j].pos)
                    } else {
                        (ida, self.balls[i].pos)
                    };
                    self.log.first_hit = Some(other);
                    self.log.first_hit_in_kitchen = at.x < HEAD_SPOT.x - 1e-9;
                }
                if vn > 0.02 {
                    self.events.push(Event::BallHit(vn));
                }
            }
        }
    }

    fn collide_cushions(&mut self) {
        let lim_x = HALF_L - R - 0.002;
        let lim_y = HALF_W - R - 0.002;
        for b in self.balls.iter_mut() {
            if !b.on_table() || !b.moving() {
                continue;
            }
            if b.pos.x.abs() < lim_x && b.pos.y.abs() < lim_y {
                continue;
            }
            for s in self.segs.iter() {
                let c = closest_on_seg(b.pos, s);
                let d = b.pos - c;
                let d2 = d.len2();
                if d2 >= R * R {
                    continue;
                }
                let dist = d2.sqrt();
                let n = if dist > 1e-9 {
                    d * (1.0 / dist)
                } else {
                    // Dead on the line: push back towards the table centre.
                    (-b.pos).norm()
                };
                b.pos = c + n * R;
                let vn = b.vel.dot(n);
                if vn >= 0.0 {
                    continue;
                }
                let e = (E_CUSHION - 0.015 * vn.abs()).max(0.6);
                let jn = -(1.0 + e) * vn;
                let t = n.perp();
                let u = b.vel.dot(t) - R * b.w[2];
                let jt = (-u / 3.5).clamp(-MU_CUSHION * jn, MU_CUSHION * jn);
                b.vel = b.vel + n * jn + t * jt;
                b.w[2] -= 2.5 / R * jt;
                // The cushion nose sits above the ball centre and kills most
                // of the roll that was carrying the ball into the rail.
                let roll = b.w[0] * t.x + b.w[1] * t.y;
                let change = (CUSHION_ROLL_KEEP - 1.0) * roll;
                b.w[0] += change * t.x;
                b.w[1] += change * t.y;

                if self.log.first_hit.is_some() {
                    self.log.cushion_after_contact = true;
                }
                if b.id != 0 && self.log.cushion_mask & (1 << b.id) == 0 {
                    self.log.cushion_mask |= 1 << b.id;
                    self.log.balls_to_cushion += 1;
                }
                if vn.abs() > 0.05 {
                    self.events.push(Event::CushionHit(vn.abs()));
                }
            }
        }
    }

    fn check_pockets(&mut self) {
        for b in self.balls.iter_mut() {
            if !b.on_table() {
                continue;
            }
            // Anything non-finite is a bug elsewhere; never let it spread.
            if !b.pos.is_finite() || !b.vel.is_finite() || b.w.iter().any(|x| !x.is_finite()) {
                b.pos = v2(
                    if b.pos.x.is_finite() {
                        b.pos.x.clamp(-HALF_L + R, HALF_L - R)
                    } else {
                        0.0
                    },
                    if b.pos.y.is_finite() {
                        b.pos.y.clamp(-HALF_W + R, HALF_W - R)
                    } else {
                        0.0
                    },
                );
                b.vel = V2::ZERO;
                b.w = [0.0; 3];
                continue;
            }
            if b.pos.x.abs() < HALF_L - 0.14 && b.pos.y.abs() < HALF_W - 0.03 {
                continue;
            }
            let mut hit = None;
            for (i, p) in self.pockets.iter().enumerate() {
                if (b.pos - p.center).len2() < p.radius * p.radius {
                    hit = Some(i);
                    break;
                }
            }
            // A ball that somehow got past the rails drops into the nearest pocket.
            if hit.is_none()
                && (b.pos.x.abs() > HALF_L + CUSHION_DEPTH
                    || b.pos.y.abs() > HALF_W + CUSHION_DEPTH)
            {
                let mut best = (0, f64::MAX);
                for (i, p) in self.pockets.iter().enumerate() {
                    let d = (b.pos - p.center).len2();
                    if d < best.1 {
                        best = (i, d);
                    }
                }
                hit = Some(best.0);
            }
            if let Some(i) = hit {
                b.state = BallState::Pocketing(i, 0.0);
                if b.id == 0 {
                    self.log.cue_potted = true;
                } else {
                    self.log.potted.push(b.id);
                    self.log.pockets.push(i);
                    // A pocketed ball counts as having been driven to a rail.
                    self.log.cushion_after_contact = true;
                }
                self.events.push(Event::Pocketed(b.id));
            }
        }
    }

    /// Run until everything stops (or `max_seconds` passes). Returns the time simulated.
    pub fn run_to_rest(&mut self, max_seconds: f64) -> f64 {
        let max_steps = (max_seconds / DT) as usize;
        for i in 0..max_steps {
            if self.at_rest() {
                return i as f64 * DT;
            }
            self.step();
            self.events.clear();
        }
        max_seconds
    }

    /// Total kinetic energy per unit mass (linear plus rotational).
    #[cfg(test)]
    pub fn energy(&self) -> f64 {
        self.balls
            .iter()
            .filter(|b| b.on_table())
            .map(|b| {
                let w2 = b.w[0] * b.w[0] + b.w[1] * b.w[1] + b.w[2] * b.w[2];
                0.5 * b.vel.len2() + 0.5 * 0.4 * R * R * w2
            })
            .sum()
    }
}

/// Cloth friction and free motion for one ball over one step.
fn integrate(b: &mut Ball, dt: f64) {
    if !b.moving() && b.w[2] == 0.0 {
        return;
    }
    // Velocity of the point of the ball touching the cloth.
    let u = v2(b.vel.x - R * b.w[1], b.vel.y + R * b.w[0]);
    let us = u.len();
    if us > 1e-7 {
        // Sliding: friction opposes the slip and changes speed and spin together.
        let k = (MU_SLIDE * G * dt).min(us / 3.5);
        let f = u * (-k / us);
        b.vel += f;
        b.w[0] += 2.5 / R * f.y;
        b.w[1] -= 2.5 / R * f.x;
    } else {
        // Rolling: a small resistance slows the ball until it stops.
        let s = b.vel.len();
        let dec = MU_ROLL * G * dt;
        if s <= dec {
            b.vel = V2::ZERO;
        } else {
            b.vel = b.vel * ((s - dec) / s);
        }
        b.w[0] = -b.vel.y / R;
        b.w[1] = b.vel.x / R;
    }
    // Spin about the vertical axis rubs off against the cloth.
    let dz = SPIN_DECAY * dt;
    if b.w[2].abs() <= dz {
        b.w[2] = 0.0;
    } else {
        b.w[2] -= dz * b.w[2].signum();
    }
    b.pos += b.vel * dt;
    let w = b.w;
    b.rotate(w, dt);
}

/// Small deterministic random number generator (xorshift64*).
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    /// Uniform in 0..1.
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in -1..1.
    pub fn signed(&mut self) -> f64 {
        self.f64() * 2.0 - 1.0
    }
    /// Roughly normal with unit variance.
    pub fn gauss(&mut self) -> f64 {
        (0..6).map(|_| self.signed()).sum::<f64>() * (0.5_f64).sqrt()
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

pub const HEAD_SPOT: V2 = v2(-HALF_L / 2.0, 0.0);
pub const FOOT_SPOT: V2 = v2(HALF_L / 2.0, 0.0);

/// Positions of a triangle rack, apex on the foot spot, row by row.
fn triangle_slots(rows: &[usize], apex_row: f64) -> Vec<V2> {
    let gap = 2.0 * R + 0.0003;
    let dx = gap * (3.0_f64).sqrt() / 2.0;
    let mut out = Vec::new();
    for (r, &n) in rows.iter().enumerate() {
        for k in 0..n {
            let y = (k as f64 - (n as f64 - 1.0) / 2.0) * gap;
            out.push(v2(FOOT_SPOT.x + (r as f64 - apex_row) * dx, y));
        }
    }
    out
}

fn add_racked(world: &mut World, order: &[u8], slots: &[V2], rng: &mut Rng) {
    world.balls.clear();
    world.balls.push(Ball::new(0, HEAD_SPOT));
    for (&id, &p) in order.iter().zip(slots.iter()) {
        // A hair of looseness so no two breaks are the same.
        let jitter = v2(rng.signed(), rng.signed()) * 0.00008;
        world.balls.push(Ball::new(id, p + jitter));
    }
    world.balls.sort_by_key(|b| b.id);
    world.log = ShotLog::default();
    world.events.clear();
}

/// Standard 8-ball rack: 8 in the middle, a solid and a stripe in the back corners.
pub fn rack_eight_ball(world: &mut World, rng: &mut Rng) {
    let slots = triangle_slots(&[1, 2, 3, 4, 5], 0.0);
    let mut solids: Vec<u8> = (1..=7).collect();
    let mut stripes: Vec<u8> = (9..=15).collect();
    rng.shuffle(&mut solids);
    rng.shuffle(&mut stripes);
    let mut order = [0u8; 15];
    order[4] = 8;
    let (c1, c2) = if rng.f64() < 0.5 { (10, 14) } else { (14, 10) };
    order[c1] = solids.pop().unwrap();
    order[c2] = stripes.pop().unwrap();
    let mut rest: Vec<u8> = solids.into_iter().chain(stripes).collect();
    rng.shuffle(&mut rest);
    for slot in order.iter_mut() {
        if *slot == 0 {
            *slot = rest.pop().unwrap();
        }
    }
    add_racked(world, &order, &slots, rng);
}

/// 9-ball diamond: 1 on the apex, 9 in the middle and on the foot spot.
pub fn rack_nine_ball(world: &mut World, rng: &mut Rng) {
    let slots = triangle_slots(&[1, 2, 3, 2, 1], 2.0);
    let mut rest: Vec<u8> = (2..=8).collect();
    rng.shuffle(&mut rest);
    let mut order = [0u8; 9];
    order[0] = 1;
    order[4] = 9;
    for slot in order.iter_mut() {
        if *slot == 0 {
            *slot = rest.pop().unwrap();
        }
    }
    add_racked(world, &order, &slots, rng);
}

/// Where a ray from `from` along unit `dir` first meets something.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    /// Distance travelled by the centre of the cue ball.
    pub dist: f64,
    /// Ball that was hit, or None for a cushion.
    pub ball: Option<u8>,
    /// Position of the cue ball centre at contact.
    pub point: V2,
}

/// Cast the cue ball along `dir` and report the first contact.
pub fn cast_cue(world: &World, from: V2, dir: V2) -> RayHit {
    let mut best = f64::MAX;
    let mut ball = None;
    for b in world.balls.iter().filter(|b| b.id != 0 && b.on_table()) {
        let oc = b.pos - from;
        let proj = oc.dot(dir);
        if proj <= 0.0 {
            continue;
        }
        let perp2 = oc.len2() - proj * proj;
        let rr = 4.0 * R * R;
        if perp2 >= rr {
            continue;
        }
        let t = proj - (rr - perp2).sqrt();
        if t >= 0.0 && t < best {
            best = t;
            ball = Some(b.id);
        }
    }
    // Rails, treated as the rectangle the ball centre can reach.
    let (lx, ly) = (HALF_L - R, HALF_W - R);
    let mut rail = f64::MAX;
    if dir.x > 1e-9 {
        rail = rail.min((lx - from.x) / dir.x);
    } else if dir.x < -1e-9 {
        rail = rail.min((-lx - from.x) / dir.x);
    }
    if dir.y > 1e-9 {
        rail = rail.min((ly - from.y) / dir.y);
    } else if dir.y < -1e-9 {
        rail = rail.min((-ly - from.y) / dir.y);
    }
    let rail = rail.max(0.0);
    if rail < best {
        best = rail;
        ball = None;
    }
    RayHit {
        dist: best,
        ball,
        point: from + dir * best,
    }
}

/// True if a ball can travel from `a` to `b` without touching any ball
/// other than the ones listed in `ignore`.
pub fn path_clear(world: &World, a: V2, b: V2, ignore: &[u8]) -> bool {
    let ab = b - a;
    let len2 = ab.len2();
    for o in world
        .balls
        .iter()
        .filter(|o| o.on_table() && !ignore.contains(&o.id))
    {
        let t = if len2 > 1e-12 {
            ((o.pos - a).dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let c = a + ab * t;
        if (o.pos - c).len2() < 4.0 * R * R {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lone_cue(pos: V2) -> World {
        let mut w = World::new();
        w.balls.push(Ball::new(0, pos));
        w
    }

    #[test]
    fn table_has_expected_geometry() {
        let w = World::new();
        assert_eq!(w.segs.len(), 20);
        assert_eq!(w.pockets.len(), 6);
    }

    #[test]
    fn ball_rolls_to_a_stop_inside_the_table() {
        let mut w = lone_cue(v2(-0.5, 0.0));
        w.strike(0.3, 3.0, 0.0, 0.0);
        let t = w.run_to_rest(60.0);
        assert!(t < 60.0, "ball never stopped");
        assert!(w.cue().on_table());
        assert!(
            World::in_bounds(w.cue().pos),
            "ball left the table: {:?}",
            w.cue().pos
        );
    }

    #[test]
    fn energy_never_increases() {
        let mut rng = Rng::new(7);
        let mut w = World::new();
        rack_eight_ball(&mut w, &mut rng);
        w.begin_shot();
        w.strike(0.01, 9.0, 0.2, 0.1);
        let mut last = w.energy();
        for _ in 0..(20.0 / DT) as usize {
            w.step();
            let e = w.energy();
            assert!(e <= last + 1e-6, "energy rose from {last} to {e}");
            last = e;
            if w.at_rest() {
                break;
            }
        }
        assert!(w.at_rest(), "break never settled");
    }

    #[test]
    fn hard_breaks_keep_every_ball_valid() {
        for seed in 0..25u64 {
            let mut rng = Rng::new(seed + 1);
            let mut w = World::new();
            rack_eight_ball(&mut w, &mut rng);
            w.begin_shot();
            w.strike(rng.signed() * 0.05, 9.0, rng.signed(), rng.signed());
            let t = w.run_to_rest(60.0);
            assert!(t < 60.0, "seed {seed} never settled");
            for b in &w.balls {
                assert!(b.pos.is_finite());
                match b.state {
                    BallState::OnTable => {
                        assert!(
                            b.pos.x.abs() <= HALF_L + CUSHION_DEPTH
                                && b.pos.y.abs() <= HALF_W + CUSHION_DEPTH,
                            "seed {seed}: ball {} escaped to {:?}",
                            b.id,
                            b.pos
                        );
                    }
                    BallState::Pocketed => {}
                    BallState::Pocketing(..) => panic!("still dropping at rest"),
                }
            }
            // No two balls left overlapping.
            for i in 0..w.balls.len() {
                for j in (i + 1)..w.balls.len() {
                    let (a, b) = (&w.balls[i], &w.balls[j]);
                    if a.on_table() && b.on_table() {
                        assert!(
                            (a.pos - b.pos).len() > 2.0 * R - 1e-4,
                            "seed {seed}: overlap"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn simulation_is_deterministic() {
        let run = || {
            let mut rng = Rng::new(42);
            let mut w = World::new();
            rack_nine_ball(&mut w, &mut rng);
            w.begin_shot();
            w.strike(0.02, 8.0, 0.3, -0.2);
            w.run_to_rest(60.0);
            w.balls
                .iter()
                .map(|b| (b.pos.x.to_bits(), b.pos.y.to_bits()))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn straight_shot_pots_the_object_ball() {
        let mut w = lone_cue(v2(0.0, 0.0));
        let target = v2(0.6, 0.3);
        w.balls.push(Ball::new(1, target));
        let pocket = w
            .pockets
            .iter()
            .find(|p| p.center.x > 1.0 && p.center.y > 0.0)
            .unwrap()
            .aim;
        let ghost = target - (pocket - target).norm() * (2.0 * R);
        w.begin_shot();
        w.strike((ghost - w.cue().pos).angle(), 2.5, 0.0, 0.0);
        w.run_to_rest(30.0);
        assert_eq!(w.log.first_hit, Some(1));
        assert_eq!(w.log.potted, vec![1]);
    }

    #[test]
    fn stop_follow_and_draw_behave() {
        // Full-ball hit: stun stops dead, follow rolls on, draw comes back.
        let shot = |top: f64| {
            let mut w = lone_cue(v2(-0.3, 0.0));
            w.balls.push(Ball::new(1, v2(0.0, 0.0)));
            w.begin_shot();
            w.strike(0.0, 1.5, 0.0, top);
            w.run_to_rest(30.0);
            w.cue().pos.x
        };
        let contact_x = -2.0 * R;
        let draw = shot(-0.9);
        let follow = shot(0.9);
        assert!(draw < contact_x - 0.1, "draw did not pull back: {draw}");
        assert!(follow > contact_x + 0.1, "follow did not run on: {follow}");
    }

    #[test]
    fn side_spin_changes_the_rebound_angle() {
        let rebound = |side: f64| {
            let mut w = lone_cue(v2(0.5, 0.0));
            w.strike(std::f64::consts::FRAC_PI_2, 2.0, side, 0.0);
            w.run_to_rest(30.0);
            w.cue().pos.x - 0.5
        };
        let plain = rebound(0.0);
        let right = rebound(0.8);
        let left = rebound(-0.8);
        assert!(plain.abs() < 0.01);
        assert!(right.abs() > 0.05 && left.abs() > 0.05);
        assert!(right * left < 0.0, "spin directions should mirror");
    }

    #[test]
    fn cast_finds_the_first_ball() {
        let mut w = lone_cue(v2(0.0, 0.0));
        w.balls.push(Ball::new(1, v2(0.5, 0.0)));
        w.balls.push(Ball::new(2, v2(0.9, 0.0)));
        let hit = cast_cue(&w, v2(0.0, 0.0), v2(1.0, 0.0));
        assert_eq!(hit.ball, Some(1));
        assert!((hit.dist - (0.5 - 2.0 * R)).abs() < 1e-9);
        let miss = cast_cue(&w, v2(0.0, 0.0), v2(0.0, 1.0));
        assert_eq!(miss.ball, None);
        assert!((miss.dist - (HALF_W - R)).abs() < 1e-9);
    }

    #[test]
    fn free_spot_search_avoids_overlap() {
        let mut w = lone_cue(HEAD_SPOT);
        w.balls.push(Ball::new(1, FOOT_SPOT));
        let p = w.find_free_spot(FOOT_SPOT, 9);
        assert!(w.spot_free(p, 9));
        assert!(World::in_bounds(p));
    }
}
