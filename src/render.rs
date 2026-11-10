//! Everything that draws: a 3D table scene, shader-lit balls, cue and UI.
//!
//! The table's surface detail (cloth, wood grain, sights, pockets) is baked
//! once per theme into a texture, then drawn as real 3D geometry: bed, raised
//! rails and cushions, apron, legs and floor. Balls are camera-facing sphere
//! impostors shaded per pixel, so a frame stays cheap on integrated graphics.

use crate::physics::{self, BallState, R, World};
use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams,
};
use macroquad::models::{Mesh, Vertex, draw_mesh};
use macroquad::prelude::*;

/// Height of the rail and cushion tops above the cloth.
pub const RAIL_H: f32 = 0.040;
/// Half extents of the area covered by the baked table texture.
const BAKE_X: f32 = 1.46;
const BAKE_Y: f32 = 0.825;
const BAKE_W: u32 = 4096;
const BAKE_H: u32 = 2314;

pub struct Theme {
    pub name: &'static str,
    pub blurb: &'static str,
    pub felt: [f32; 3],
    pub wood1: [f32; 3],
    pub wood2: [f32; 3],
    pub accent: [f32; 3],
    pub floor: [f32; 3],
    /// Colour of the light strips on the futuristic tables (black = none).
    pub glow: [f32; 3],
}

pub const THEMES: [Theme; 12] = [
    Theme {
        name: "Classic Club",
        blurb: "Green worsted cloth on mahogany",
        felt: [0.05, 0.40, 0.21],
        wood1: [0.40, 0.15, 0.08],
        wood2: [0.20, 0.06, 0.03],
        accent: [0.94, 0.90, 0.80],
        floor: [0.13, 0.09, 0.07],
        glow: [0.0, 0.0, 0.0],
    },
    Theme {
        name: "Tournament Blue",
        blurb: "Broadcast blue on piano black",
        felt: [0.06, 0.30, 0.60],
        wood1: [0.10, 0.10, 0.11],
        wood2: [0.035, 0.035, 0.04],
        accent: [0.86, 0.88, 0.92],
        floor: [0.07, 0.08, 0.11],
        glow: [0.0, 0.0, 0.0],
    },
    Theme {
        name: "Burgundy Lounge",
        blurb: "Wine red cloth on dark walnut",
        felt: [0.47, 0.07, 0.12],
        wood1: [0.34, 0.21, 0.12],
        wood2: [0.16, 0.09, 0.05],
        accent: [0.96, 0.78, 0.40],
        floor: [0.10, 0.07, 0.08],
        glow: [0.0, 0.0, 0.0],
    },
    Theme {
        name: "Royal Oak",
        blurb: "Teal cloth on honey oak",
        felt: [0.05, 0.36, 0.34],
        wood1: [0.66, 0.45, 0.24],
        wood2: [0.44, 0.27, 0.12],
        accent: [0.16, 0.10, 0.06],
        floor: [0.12, 0.10, 0.08],
        glow: [0.0, 0.0, 0.0],
    },
    Theme {
        name: "Midnight Carbon",
        blurb: "Graphite cloth on matte carbon",
        felt: [0.20, 0.23, 0.28],
        wood1: [0.13, 0.13, 0.15],
        wood2: [0.06, 0.06, 0.07],
        accent: [0.30, 0.85, 0.95],
        floor: [0.05, 0.05, 0.06],
        glow: [0.0, 0.0, 0.0],
    },
    Theme {
        name: "Neon Grid",
        blurb: "Futuristic: cyan light rails and a holo-grid cloth",
        felt: [0.03, 0.13, 0.20],
        wood1: [0.07, 0.08, 0.10],
        wood2: [0.02, 0.02, 0.03],
        accent: [0.40, 0.95, 1.00],
        floor: [0.03, 0.05, 0.07],
        glow: [0.10, 0.90, 1.00],
    },
    Theme {
        name: "Plasma Arena",
        blurb: "Futuristic: magenta plasma on violet cloth",
        felt: [0.17, 0.06, 0.26],
        wood1: [0.08, 0.06, 0.10],
        wood2: [0.02, 0.02, 0.03],
        accent: [1.00, 0.55, 0.95],
        floor: [0.06, 0.03, 0.07],
        glow: [1.00, 0.20, 0.85],
    },
    Theme {
        name: "Solar Flare",
        blurb: "Futuristic: amber light on graphite cloth",
        felt: [0.10, 0.11, 0.12],
        wood1: [0.10, 0.09, 0.08],
        wood2: [0.03, 0.03, 0.03],
        accent: [1.00, 0.78, 0.35],
        floor: [0.06, 0.045, 0.03],
        glow: [1.00, 0.55, 0.10],
    },
    Theme {
        name: "Obsidian Lime",
        blurb: "Charcoal + slate / electric lime",
        felt: [0.105, 0.145, 0.155],
        wood1: [0.22, 0.27, 0.28],
        wood2: [0.055, 0.075, 0.080],
        accent: [0.85, 1.00, 0.16],
        floor: [0.045, 0.055, 0.060],
        glow: [0.54, 0.83, 0.05],
    },
    Theme {
        name: "Cobalt Ember",
        blurb: "Midnight blue + steel / vivid orange",
        felt: [0.050, 0.145, 0.265],
        wood1: [0.17, 0.29, 0.43],
        wood2: [0.030, 0.065, 0.115],
        accent: [1.00, 0.49, 0.16],
        floor: [0.035, 0.050, 0.080],
        glow: [0.93, 0.28, 0.045],
    },
    Theme {
        name: "Violet Citron",
        blurb: "Deep plum + mauve / citrus yellow",
        felt: [0.19, 0.105, 0.275],
        wood1: [0.32, 0.22, 0.39],
        wood2: [0.075, 0.045, 0.100],
        accent: [0.92, 1.00, 0.23],
        floor: [0.065, 0.045, 0.075],
        glow: [0.70, 0.82, 0.085],
    },
    Theme {
        name: "Sea Glass Coral",
        blurb: "Deep teal + sea glass / luminous coral",
        felt: [0.035, 0.22, 0.23],
        wood1: [0.15, 0.36, 0.35],
        wood2: [0.025, 0.085, 0.085],
        accent: [1.00, 0.42, 0.34],
        floor: [0.035, 0.065, 0.065],
        glow: [0.93, 0.27, 0.20],
    },
];

/// A perspective camera plus the helpers the game needs to work in 3D.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cam {
    pub pos: Vec3,
    pub target: Vec3,
    /// Vertical field of view in radians.
    pub fov: f32,
}

impl Cam {
    /// Straight down on the table, framed to fit the window.
    pub fn top(aspect: f32) -> Cam {
        let fov = 30.0_f32.to_radians();
        let t = (fov / 2.0).tan();
        let h = (0.80 * 1.36 / t).max(1.47 * 1.03 / (aspect * t));
        Cam {
            pos: vec3(0.0, 0.012, h),
            target: vec3(0.0, 0.012, 0.0),
            fov,
        }
    }

    /// Broadcast-style view of the whole table from the long side.
    pub fn overview() -> Cam {
        Cam {
            pos: vec3(0.0, -2.25, 1.80),
            target: vec3(0.0, -0.10, 0.0),
            fov: 42.0_f32.to_radians(),
        }
    }

    pub fn lerp(self, o: Cam, k: f32) -> Cam {
        Cam {
            pos: self.pos.lerp(o.pos, k),
            target: self.target.lerp(o.target, k),
            fov: self.fov + (o.fov - self.fov) * k,
        }
    }

    fn up(&self) -> Vec3 {
        let d = (self.target - self.pos).normalize_or_zero();
        if d.z.abs() > 0.995 {
            vec3(0.0, 1.0, 0.0)
        } else {
            vec3(0.0, 0.0, 1.0)
        }
    }

    /// Camera right, camera up and the direction towards the viewer.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = (self.target - self.pos).normalize_or_zero();
        let r = f.cross(self.up()).normalize_or_zero();
        (r, r.cross(f), -f)
    }

    fn view_proj(&self) -> Mat4 {
        let aspect = screen_width() / screen_height().max(1.0);
        Mat4::perspective_rh_gl(self.fov, aspect, 0.05, 60.0)
            * Mat4::look_at_rh(self.pos, self.target, self.up())
    }

    fn camera3d(&self) -> Camera3D {
        Camera3D {
            position: self.pos,
            target: self.target,
            up: self.up(),
            fovy: self.fov,
            z_near: 0.05,
            z_far: 60.0,
            ..Default::default()
        }
    }

    /// Screen position of a world point, if it is in front of the camera.
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let c = self.view_proj() * p.extend(1.0);
        if c.w <= 0.02 {
            return None;
        }
        Some(vec2(
            (c.x / c.w + 1.0) * 0.5 * screen_width(),
            (1.0 - c.y / c.w) * 0.5 * screen_height(),
        ))
    }

    /// Screen position of a point on the table at ball-centre height.
    pub fn to_screen(&self, p: physics::V2) -> Vec2 {
        self.project(at(p, R as f32))
            .unwrap_or(vec2(-9999.0, -9999.0))
    }

    /// Screen pixels per metre at a world point.
    pub fn px_per_m(&self, p: Vec3) -> f32 {
        let (r, _, _) = self.basis();
        match (self.project(p), self.project(p + r * 0.1)) {
            (Some(a), Some(b)) => (b - a).length() * 10.0,
            _ => 0.0,
        }
    }

    /// Where the ray through a screen point meets the plane of ball centres.
    pub fn to_world(&self, s: Vec2) -> physics::V2 {
        let inv = self.view_proj().inverse();
        let n = vec2(
            s.x / screen_width() * 2.0 - 1.0,
            1.0 - s.y / screen_height() * 2.0,
        );
        let a = inv * vec4(n.x, n.y, -1.0, 1.0);
        let b = inv * vec4(n.x, n.y, 1.0, 1.0);
        let (a, b) = (a.truncate() / a.w, b.truncate() / b.w);
        let d = b - a;
        let t = if d.z.abs() > 1e-6 {
            (R as f32 - a.z) / d.z
        } else {
            0.0
        };
        let p = a + d * t.max(0.0);
        physics::v2(p.x as f64, p.y as f64)
    }
}

/// A table position lifted to height `z`.
pub fn at(p: physics::V2, z: f32) -> Vec3 {
    vec3(p.x as f32, p.y as f32, z)
}

const VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
varying highp vec2 uv;
varying lowp vec4 color;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    color = color0 / 255.0;
    uv = texcoord;
}
"#;

const TABLE_FRAG: &str = r#"#version 100
precision highp float;
varying highp vec2 uv;
uniform vec2 u_res;
uniform vec4 u_view;
uniform vec3 u_felt;
uniform vec3 u_wood1;
uniform vec3 u_wood2;
uniform vec3 u_accent;
uniform vec3 u_glow;

const float L = 1.27;
const float W = 0.635;
const float CD = 0.05;
const float RW = 0.115;
const float DC = 0.0863;
const float DS = 0.0675;

float hash(vec2 p) {
    p = fract(p * vec2(0.1031, 0.1030));
    p += dot(p, p.yx + 33.33);
    return fract((p.x + p.y) * p.x);
}
float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}
float fbm(vec2 p) {
    float a = 0.5;
    float s = 0.0;
    for (int i = 0; i < 4; i++) {
        s += a * noise(p);
        p = p * 2.03 + 17.1;
        a *= 0.5;
    }
    return s;
}
float sdBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
float lamp(vec2 w, float x) {
    vec2 d = w - vec2(x, 0.0);
    return 1.0 / (1.0 + dot(d, d) * 2.4);
}

void main() {
    vec2 px = uv * u_res;
    vec2 w = vec2(px.x - u_view.x, u_view.y - px.y) / u_view.z;
    float aa = 1.2 / u_view.z;
    vec2 q = abs(w);
    float li = (lamp(w, -0.75) + lamp(w, 0.0) + lamp(w, 0.75)) / 1.85;

    vec3 col = u_wood2 * 0.5;
    float alpha = 0.0;
    vec2 outer = vec2(L + CD + RW, W + CD + RW);
    float dOut = sdBox(w, outer, 0.09);

    if (dOut < aa) {
        vec3 t;
        float ex = q.x - (L + CD);
        float ey = q.y - (W + CD);
        bool onWood = max(ex, ey) > 0.0;
        bool raised = onWood;
        if (onWood) {
            bool horiz = ey > ex;
            vec2 g = horiz ? vec2(w.x * 2.2, w.y * 46.0) : vec2(w.y * 2.2 + 31.0, w.x * 46.0);
            float grain = fbm(g + fbm(g * 0.6) * 2.5);
            float fine = noise(g * vec2(3.0, 9.0));
            t = mix(u_wood2, u_wood1, smoothstep(0.22, 0.72, grain));
            t *= 0.86 + 0.28 * fine;
            // Mitre joint where the rails meet.
            t *= 0.72 + 0.28 * smoothstep(0.0, 0.004, abs(ey - ex));
            // Rounded profile: dark outer edge, gloss streak, lip at the cushion.
            float k = clamp(-dOut / RW, 0.0, 1.0);
            t *= 0.55 + 0.55 * smoothstep(0.0, 0.16, k) - 0.22 * smoothstep(0.86, 1.0, k);
            t += vec3(0.11) * pow(max(1.0 - abs(k - 0.36) * 3.2, 0.0), 3.0) * (0.5 + 0.8 * li);
            t *= 0.62 + 0.6 * li;
            // Light strips on the futuristic tables.
            t += u_glow * (smoothstep(0.035, 0.0, abs(k - 0.93)) + smoothstep(0.03, 0.0, abs(k - 0.07))) * 1.5;
            // Sights.
            vec2 dd;
            if (horiz) {
                float sx = min(min(abs(q.x - 0.3175), abs(q.x - 0.635)), abs(q.x - 0.9525));
                dd = vec2(sx / 0.013, (q.y - (W + CD + RW * 0.5)) / 0.0075);
            } else {
                float sy = min(q.y, abs(q.y - 0.3175));
                dd = vec2((q.x - (L + CD + RW * 0.5)) / 0.0075, sy / 0.013);
            }
            float dia = abs(dd.x) + abs(dd.y) - 1.0;
            vec3 pearl = u_accent * (0.75 + 0.35 * li);
            t = mix(t, pearl + u_glow * 1.2, smoothstep(aa * 90.0, 0.0, dia));
        } else {
            // Cloth.
            vec3 felt = u_felt;
            felt *= 0.90 + 0.14 * noise(w * 1100.0);
            felt *= 0.94 + 0.10 * noise(vec2(w.x * 130.0, w.y * 700.0));
            felt *= 0.93 + 0.14 * fbm(w * 5.0);
            float lit = 0.46 + 0.80 * li;
            // Cushions, mirrored into one quadrant.
            float dTop = max(max(W - q.y, q.y - (W + CD)),
                             max(-dot(q - vec2(DS, W), vec2(0.9703, 0.2419)),
                                 -dot(q - vec2(L - DC, W), vec2(-0.6157, 0.7880))));
            float dRight = max(max(L - q.x, q.x - (L + CD)),
                               -dot(q - vec2(L, W - DC), vec2(0.7880, -0.6157)));
            float dcu = min(dTop, dRight);
            if (dcu < 0.0) {
                raised = true;
                float u = (dTop < dRight) ? (q.y - W) / CD : (q.x - L) / CD;
                t = felt * (1.22 - 0.50 * u) * lit;
                t += u_felt * 0.20 * smoothstep(0.16, 0.0, u) * lit;
                t *= 0.80 + 0.20 * smoothstep(0.0, 0.004, -dcu);
            } else {
                t = felt * lit;
                t *= 1.0 - 0.42 * smoothstep(0.035, 0.0, dcu);
                // Spots and head string.
                float spot = length(vec2(q.x - L * 0.5, w.y));
                t *= 1.0 - 0.45 * smoothstep(0.0075 + aa, 0.0075, spot);
                t *= 1.0 + 0.10 * smoothstep(0.0022 + aa, 0.0010, abs(w.x + L * 0.5)) * step(q.y, W);
                vec2 cellp = abs(fract(w / 0.15875 + 0.5) - 0.5) * 0.15875;
                t += u_glow * smoothstep(0.0022, 0.0004, min(cellp.x, cellp.y)) * 0.08 * step(q.x, L) * step(q.y, W);
            }
        }
        // Pockets.
        float dh = min(length(q - vec2(L + 0.02, W + 0.02)) - 0.07,
                       length(q - vec2(0.0, W + 0.04)) - 0.062);
        if (onWood) {
            vec3 liner = vec3(0.055, 0.05, 0.048) * (0.7 + 0.6 * li);
            liner += vec3(0.05) * smoothstep(0.012, 0.022, dh);
            liner += u_glow * smoothstep(0.020, 0.004, dh) * 1.3;
            t = mix(t, liner, smoothstep(0.024 + aa, 0.024, dh));
        }
        vec3 hole = vec3(0.004) + vec3(0.035) * smoothstep(-0.028, 0.0, dh);
        t = mix(t, hole, smoothstep(aa, 0.0, dh));
        col = t;
        alpha = (raised && dh > 0.0) ? 1.0 : 0.5;
    }
    if (dOut > 0.0) alpha = 0.0;
    gl_FragColor = vec4(col, alpha);
}
"#;

const SURFACE_FRAG: &str = r#"#version 100
precision highp float;
varying highp vec2 uv;
uniform sampler2D Texture;
uniform vec4 u_surf; // x: 0 = bed pass, 1 = raised pass
void main() {
    vec4 t = texture2D(Texture, uv);
    if (t.a < (u_surf.x > 0.5 ? 0.75 : 0.25)) discard;
    gl_FragColor = vec4(t.rgb, 1.0);
}
"#;

const BALL_FRAG: &str = r#"#version 100
precision highp float;
varying highp vec2 uv;
uniform sampler2D Texture;
uniform vec3 u_c0;
uniform vec3 u_c1;
uniform vec3 u_c2;
uniform vec3 u_color;
uniform vec3 u_felt;
uniform vec4 u_params; // kind, number, quad scale, pixel size
uniform vec4 u_pos;    // world x, world y, brightness, alpha
uniform vec3 u_bx;
uniform vec3 u_by;
uniform vec3 u_bz;

vec2 lampLight(vec3 n, float lx) {
    vec3 lv = normalize(vec3(lx - u_pos.x, -u_pos.y, 0.85));
    float nl = max(dot(n, lv), 0.0);
    vec3 h = normalize(lv + u_bz);
    float nh = max(dot(n, h), 0.0);
    return vec2(nl, pow(nh, 260.0) + 0.05 * pow(nh, 18.0));
}

vec3 ink(vec2 nuv, float num) {
    vec2 cell = vec2(mod(num, 4.0), floor(num / 4.0));
    vec2 c = (cell + clamp(nuv, 0.03, 0.97)) / 4.0;
    float o = u_params.w * 0.06;
    vec3 s = texture2D(Texture, c + vec2(o, o)).rgb + texture2D(Texture, c + vec2(-o, o)).rgb
           + texture2D(Texture, c + vec2(o, -o)).rgb + texture2D(Texture, c + vec2(-o, -o)).rgb;
    return s * 0.25;
}

void main() {
    vec2 p = (uv * 2.0 - 1.0) * u_params.z;
    float r = length(p);
    float px = u_params.w;
    float edge = 1.0 - smoothstep(1.0 - 1.6 * px, 1.0, r);
    if (edge <= 0.0) discard;
    float z = sqrt(max(1.0 - r * r, 0.0));
    vec3 n = u_bx * p.x + u_by * p.y + u_bz * z;
    vec3 o = vec3(dot(u_c0, n), dot(u_c1, n), dot(u_c2, n));
    float kind = u_params.x;
    float aw = px * 1.2 + 0.008;
    vec3 white = vec3(0.97, 0.95, 0.88);
    vec3 base = u_color;
    if (kind > 1.5) {
        base = mix(white, u_color, smoothstep(0.50 + aw, 0.50 - aw, abs(o.y)));
    }
    if (kind > 0.5) {
        float d = length(o.xy);
        vec2 nuv = o.xy / 0.43;
        if (o.z < 0.0) nuv.x = -nuv.x;
        nuv = nuv * 0.5 + 0.5;
        nuv.y = 1.0 - nuv.y;
        base = mix(base, white * ink(nuv, u_params.y), smoothstep(0.41 + aw, 0.41 - aw, d));
    } else {
        base = white;
        vec3 a = abs(o);
        float m = max(a.x, max(a.y, a.z));
        float d = sqrt(max(1.0 - m * m, 0.0));
        base = mix(base, vec3(0.78, 0.07, 0.07), smoothstep(0.125 + aw, 0.125 - aw, d));
    }
    vec2 l = lampLight(n, -0.75) + lampLight(n, 0.0) + lampLight(n, 0.75);
    vec3 col = base * (0.33 + 0.36 * l.x);
    float down = clamp(0.5 - 0.5 * n.z, 0.0, 1.0);
    col += base * u_felt * 0.75 * down * down;
    col += vec3(1.0, 0.98, 0.94) * l.y * 0.95;
    float fres = pow(1.0 - z, 4.0);
    col = mix(col, col * 0.45 + vec3(0.07), fres * 0.7);
    gl_FragColor = vec4(col * u_pos.z, edge * u_pos.w);
}
"#;

const SHADOW_FRAG: &str = r#"#version 100
precision highp float;
varying highp vec2 uv;
varying lowp vec4 color;
void main() {
    float d = length(uv * 2.0 - 1.0);
    float a = smoothstep(1.0, 0.2, d);
    gl_FragColor = vec4(color.rgb, color.a * a * a);
}
"#;

const PANEL_FRAG: &str = r#"#version 100
precision highp float;
varying highp vec2 uv;
uniform vec2 u_size;
uniform vec4 u_fill;
uniform vec4 u_border;
uniform vec2 u_rb; // corner radius, border width
void main() {
    vec2 p = (uv - 0.5) * u_size;
    vec2 q = abs(p) - u_size * 0.5 + u_rb.x;
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_rb.x;
    float a = clamp(0.5 - d, 0.0, 1.0);
    float b = clamp(0.5 + d + u_rb.y, 0.0, 1.0) * step(0.01, u_rb.y);
    vec4 c = mix(u_fill, u_border, b * u_border.a);
    c.rgb *= 1.0 + 0.10 * (0.5 - uv.y);
    gl_FragColor = vec4(c.rgb, mix(u_fill.a, max(u_fill.a, u_border.a), b) * a);
}
"#;

fn pipeline(depth: bool, write: bool, blend: bool) -> PipelineParams {
    PipelineParams {
        color_blend: blend.then(|| {
            BlendState::new(
                Equation::Add,
                BlendFactor::Value(BlendValue::SourceAlpha),
                BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
            )
        }),
        depth_test: if depth {
            Comparison::LessOrEqual
        } else {
            Comparison::Always
        },
        depth_write: write,
        ..Default::default()
    }
}

fn material(
    fragment: &str,
    uniforms: Vec<UniformDesc>,
    pipeline_params: PipelineParams,
) -> Material {
    load_material(
        ShaderSource::Glsl {
            vertex: VERTEX,
            fragment,
        },
        MaterialParams {
            pipeline_params,
            uniforms,
            textures: vec![],
        },
    )
    .expect("shader failed to compile")
}

pub fn ball_color(id: u8) -> [f32; 3] {
    match if id > 8 { id - 8 } else { id } {
        1 => [0.98, 0.76, 0.06],
        2 => [0.06, 0.18, 0.62],
        3 => [0.84, 0.07, 0.07],
        4 => [0.36, 0.11, 0.52],
        5 => [0.96, 0.40, 0.04],
        6 => [0.03, 0.38, 0.19],
        7 => [0.46, 0.07, 0.10],
        8 => [0.03, 0.03, 0.035],
        _ => [0.97, 0.95, 0.88],
    }
}

fn rgb(c: [f32; 3]) -> Vec3 {
    vec3(c[0], c[1], c[2])
}

fn shade(c: [f32; 3], k: f32) -> Color {
    Color::new(
        (c[0] * k).min(1.0),
        (c[1] * k).min(1.0),
        (c[2] * k).min(1.0),
        1.0,
    )
}

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn empty_mesh(texture: Option<Texture2D>) -> Mesh {
    Mesh {
        vertices: Vec::new(),
        indices: Vec::new(),
        texture,
    }
}

/// Append a quad (corners in order around the edge) to the last mesh.
fn push_quad(meshes: &mut Vec<Mesh>, p: [Vec3; 4], uv: [Vec2; 4], c: [Color; 4]) {
    if meshes.last().map_or(true, |m| m.indices.len() > 4000) {
        meshes.push(empty_mesh(None));
    }
    let m = meshes.last_mut().unwrap();
    let base = m.vertices.len() as u16;
    for k in 0..4 {
        m.vertices.push(Vertex::new2(p[k], uv[k], c[k]));
    }
    m.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn quad_mesh(p: [Vec3; 4], uv: [Vec2; 4], c: Color, texture: Option<Texture2D>) -> Mesh {
    let mut v = vec![empty_mesh(texture)];
    push_quad(&mut v, p, uv, [c; 4]);
    v.pop().unwrap()
}

const UV: [Vec2; 4] = [
    Vec2::new(0.0, 0.0),
    Vec2::new(1.0, 0.0),
    Vec2::new(1.0, 1.0),
    Vec2::new(0.0, 1.0),
];

pub struct Renderer {
    bake_mat: Material,
    surface_mat: Material,
    ball3d: Material,
    ball2d: Material,
    shadow_mat: Material,
    panel_mat: Material,
    white: Texture2D,
    atlas: RenderTarget,
    bake: RenderTarget,
    baked_theme: Option<usize>,
    statics: Vec<Mesh>,
    bed: Mesh,
    top: Mesh,
    font: Option<Font>,
    font_bold: Option<Font>,
    pub theme: usize,
}

fn load_font(names: &[&str]) -> Option<Font> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    for name in names {
        if let Ok(bytes) = std::fs::read(format!("{dir}\\Fonts\\{name}")) {
            if let Ok(mut f) = load_ttf_font_from_bytes(&bytes) {
                f.set_filter(FilterMode::Linear);
                return Some(f);
            }
        }
    }
    None
}

impl Renderer {
    pub fn new() -> Renderer {
        let f3 = |n: &str| UniformDesc::new(n, UniformType::Float3);
        let f4 = |n: &str| UniformDesc::new(n, UniformType::Float4);
        let f2 = |n: &str| UniformDesc::new(n, UniformType::Float2);
        let bake_mat = material(
            TABLE_FRAG,
            vec![
                f2("u_res"),
                f4("u_view"),
                f3("u_felt"),
                f3("u_wood1"),
                f3("u_wood2"),
                f3("u_accent"),
                f3("u_glow"),
            ],
            pipeline(false, false, false),
        );
        let surface_mat = material(
            SURFACE_FRAG,
            vec![f4("u_surf")],
            pipeline(true, true, false),
        );
        let ball_uniforms = || {
            vec![
                f3("u_c0"),
                f3("u_c1"),
                f3("u_c2"),
                f3("u_color"),
                f3("u_felt"),
                f4("u_params"),
                f4("u_pos"),
                f3("u_bx"),
                f3("u_by"),
                f3("u_bz"),
            ]
        };
        let ball3d = material(BALL_FRAG, ball_uniforms(), pipeline(true, true, true));
        let ball2d = material(BALL_FRAG, ball_uniforms(), pipeline(false, false, true));
        let shadow_mat = material(SHADOW_FRAG, vec![], pipeline(true, false, true));
        let panel_mat = material(
            PANEL_FRAG,
            vec![f2("u_size"), f4("u_fill"), f4("u_border"), f2("u_rb")],
            pipeline(false, false, true),
        );
        let white = Texture2D::from_rgba8(1, 1, &[255, 255, 255, 255]);
        let font = load_font(&["segoeui.ttf", "arial.ttf", "tahoma.ttf"]);
        let font_bold =
            load_font(&["seguisb.ttf", "segoeuib.ttf", "arialbd.ttf"]).or_else(|| font.clone());

        // Number decals for the object balls, 4 x 4 cells.
        let cell = 64.0;
        let atlas = render_target(256, 256);
        atlas.texture.set_filter(FilterMode::Linear);
        let cam = Camera2D {
            target: vec2(128.0, 128.0),
            zoom: vec2(2.0 / 256.0, 2.0 / 256.0),
            render_target: Some(atlas.clone()),
            ..Default::default()
        };
        set_camera(&cam);
        clear_background(WHITE);
        for i in 1..=15u32 {
            let s = i.to_string();
            let size = if i < 10 { 54 } else { 44 };
            let dim = measure_text(&s, font_bold.as_ref(), size, 1.0);
            let cx = (i % 4) as f32 * cell + cell / 2.0;
            let cy = (i / 4) as f32 * cell + cell / 2.0;
            draw_text_ex(
                &s,
                (cx - dim.width / 2.0).round(),
                (cy - dim.height / 2.0 + dim.offset_y).round(),
                TextParams {
                    font: font_bold.as_ref(),
                    font_size: size,
                    color: Color::new(0.03, 0.03, 0.03, 1.0),
                    ..Default::default()
                },
            );
            if i == 6 || i == 9 {
                draw_rectangle(
                    cx - 11.0,
                    cy + dim.height / 2.0 + 3.0,
                    22.0,
                    4.0,
                    Color::new(0.03, 0.03, 0.03, 1.0),
                );
            }
        }
        set_default_camera();

        let bake = render_target(BAKE_W, BAKE_H);
        bake.texture.set_filter(FilterMode::Linear);
        let corners = |z: f32| {
            [
                vec3(-BAKE_X, BAKE_Y, z),
                vec3(BAKE_X, BAKE_Y, z),
                vec3(BAKE_X, -BAKE_Y, z),
                vec3(-BAKE_X, -BAKE_Y, z),
            ]
        };
        let bed = quad_mesh(corners(0.0), UV, WHITE, Some(bake.texture.clone()));
        let top = quad_mesh(corners(RAIL_H), UV, WHITE, Some(bake.texture.clone()));

        Renderer {
            bake_mat,
            surface_mat,
            ball3d,
            ball2d,
            shadow_mat,
            panel_mat,
            white,
            atlas,
            bake,
            baked_theme: None,
            statics: Vec::new(),
            bed,
            top,
            font,
            font_bold,
            theme: 0,
        }
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[self.theme % THEMES.len()]
    }

    /// Bake the surface texture and rebuild the room for the current theme.
    fn ensure_baked(&mut self) {
        if self.baked_theme == Some(self.theme) {
            return;
        }
        self.baked_theme = Some(self.theme);
        let th = self.theme();
        let (w, h) = (BAKE_W as f32, BAKE_H as f32);
        let cam = Camera2D {
            target: vec2(w / 2.0, h / 2.0),
            zoom: vec2(2.0 / w, 2.0 / h),
            render_target: Some(self.bake.clone()),
            ..Default::default()
        };
        set_camera(&cam);
        let m = &self.bake_mat;
        m.set_uniform("u_res", vec2(w, h));
        m.set_uniform("u_view", vec4(w / 2.0, h / 2.0, w / (2.0 * BAKE_X), 0.0));
        m.set_uniform("u_felt", rgb(th.felt));
        m.set_uniform("u_wood1", rgb(th.wood1));
        m.set_uniform("u_wood2", rgb(th.wood2));
        m.set_uniform("u_accent", rgb(th.accent));
        m.set_uniform("u_glow", rgb(th.glow));
        gl_use_material(m);
        draw_texture_ex(
            &self.white,
            0.0,
            0.0,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(w, h)),
                ..Default::default()
            },
        );
        gl_use_default_material();
        set_default_camera();

        let mut v: Vec<Mesh> = Vec::new();
        let z2 = [Vec2::ZERO; 4];
        // Floor, lit in a pool around the table.
        let (n, span, fz) = (16, 9.0f32, -0.78f32);
        let fcol = |x: f32, y: f32| {
            let pool = (-(x * x * 0.5 + y * y) * 0.16).exp();
            let under = (-(x * x * 0.35 + y * y) * 0.55).exp() * 0.30;
            let k = 0.22 + 1.5 * pool;
            Color::new(
                th.floor[0] * k + th.glow[0] * under,
                th.floor[1] * k + th.glow[1] * under,
                th.floor[2] * k + th.glow[2] * under,
                1.0,
            )
        };
        for i in 0..n {
            for j in 0..n {
                let g = |k: i32| (k as f32 / n as f32 * 2.0 - 1.0) * span;
                let (x0, x1, y0, y1) = (g(i), g(i + 1), g(j), g(j + 1));
                push_quad(
                    &mut v,
                    [
                        vec3(x0, y0, fz),
                        vec3(x1, y0, fz),
                        vec3(x1, y1, fz),
                        vec3(x0, y1, fz),
                    ],
                    z2,
                    [fcol(x0, y0), fcol(x1, y0), fcol(x1, y1), fcol(x0, y1)],
                );
            }
        }
        // A vertical wall along a line, from height `za` down to `zb`.
        let wall = |v: &mut Vec<Mesh>, a: Vec2, b: Vec2, za: f32, zb: f32, ca: Color, cb: Color| {
            push_quad(
                v,
                [a.extend(za), b.extend(za), b.extend(zb), a.extend(zb)],
                z2,
                [ca, ca, cb, cb],
            );
        };
        // Legs.
        for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let c = vec2(sx * 1.16, sy * 0.56);
            let k = [
                vec2(-0.075, -0.075),
                vec2(0.075, -0.075),
                vec2(0.075, 0.075),
                vec2(-0.075, 0.075),
            ];
            for e in 0..4 {
                let lit = if e == 0 { 0.50 } else { 0.30 };
                wall(
                    &mut v,
                    c + k[e],
                    c + k[(e + 1) % 4],
                    -0.2,
                    fz,
                    shade(th.wood2, lit),
                    shade(th.wood2, lit * 0.5),
                );
            }
        }
        // Apron around the outside of the rails.
        let (ox, oy, rad) = (1.435f32, 0.80f32, 0.09f32);
        let mut ring: Vec<Vec2> = Vec::new();
        for (q, (sx, sy)) in [(1.0f32, 1.0f32), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
            .iter()
            .enumerate()
        {
            let c = vec2(sx * (ox - rad), sy * (oy - rad));
            for s in 0..=8 {
                let a = (q as f32 + s as f32 / 8.0) * std::f32::consts::FRAC_PI_2;
                ring.push(c + vec2(a.cos(), a.sin()) * rad);
            }
        }
        for k in 0..ring.len() {
            let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
            let nrm = (b - a).perp().normalize_or_zero();
            let lit = 0.50 + 0.22 * nrm.y.abs();
            wall(
                &mut v,
                a,
                b,
                RAIL_H,
                -0.2,
                shade(th.wood1, lit),
                shade(th.wood2, lit * 0.45),
            );
            if th.glow.iter().sum::<f32>() > 0.0 {
                let g = shade(th.glow, 1.0);
                wall(&mut v, a * 1.003, b * 1.003, -0.165, -0.2, g, g);
            }
        }
        // Cushion faces along the same lines the physics uses.
        for s in World::new().segs.iter() {
            let (a, b) = (
                vec2(s.a.x as f32, s.a.y as f32),
                vec2(s.b.x as f32, s.b.y as f32),
            );
            wall(
                &mut v,
                a,
                b,
                RAIL_H,
                0.0,
                shade(th.felt, 0.62),
                shade(th.felt, 0.30),
            );
        }
        self.statics = v;
    }

    #[allow(clippy::too_many_arguments)]
    fn set_ball_uniforms(
        &self,
        m: &Material,
        id: u8,
        cols: [[f32; 3]; 3],
        world: (f32, f32),
        bright: f32,
        alpha: f32,
        scale: f32,
        px: f32,
        basis: (Vec3, Vec3, Vec3),
    ) {
        let kind = if id == 0 {
            0.0
        } else if id <= 8 {
            1.0
        } else {
            2.0
        };
        m.set_uniform("u_c0", rgb(cols[0]));
        m.set_uniform("u_c1", rgb(cols[1]));
        m.set_uniform("u_c2", rgb(cols[2]));
        m.set_uniform("u_color", rgb(ball_color(id)));
        m.set_uniform("u_felt", rgb(self.theme().felt));
        m.set_uniform("u_params", vec4(kind, id as f32, scale, px));
        m.set_uniform("u_pos", vec4(world.0, world.1, bright, alpha));
        m.set_uniform("u_bx", basis.0);
        m.set_uniform("u_by", basis.1);
        m.set_uniform("u_bz", basis.2);
    }

    /// A ball facing the viewer, for the HUD.
    pub fn draw_icon_ball(&self, at: Vec2, radius: f32, id: u8, alpha: f32) {
        if radius < 0.5 || alpha <= 0.0 {
            return;
        }
        let dpi = screen_dpi_scale().max(1.0);
        let half = radius + 1.5;
        let basis = (
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, -1.0, 0.0),
            vec3(0.0, 0.0, 1.0),
        );
        self.set_ball_uniforms(
            &self.ball2d,
            id,
            IDENTITY,
            (0.0, 0.25),
            1.0,
            alpha,
            half / radius,
            1.0 / (radius * dpi),
            basis,
        );
        gl_use_material(&self.ball2d);
        draw_texture_ex(
            &self.atlas.texture,
            at.x - half,
            at.y - half,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(half * 2.0, half * 2.0)),
                ..Default::default()
            },
        );
        gl_use_default_material();
    }

    /// Draw the room, the table and every ball from `cam`. `lead` extrapolates
    /// moving balls by the part of a physics step not simulated yet.
    pub fn draw_scene(&mut self, cam: &Cam, world: &World, lead: f64) {
        self.ensure_baked();
        set_camera(&cam.camera3d());
        for m in &self.statics {
            draw_mesh(m);
        }
        self.surface_mat
            .set_uniform("u_surf", vec4(0.0, 0.0, 0.0, 0.0));
        gl_use_material(&self.surface_mat);
        draw_mesh(&self.bed);
        self.surface_mat
            .set_uniform("u_surf", vec4(1.0, 0.0, 0.0, 0.0));
        draw_mesh(&self.top);

        let r = R as f32;
        let place = |b: &physics::Ball| -> Option<(Vec3, f32)> {
            let p = if b.on_table() {
                b.pos + b.vel * lead
            } else {
                b.pos
            };
            match b.state {
                BallState::OnTable => Some((at(p, r), 1.0)),
                BallState::Pocketing(_, t) => {
                    let k = (t / physics::POCKET_TIME) as f32;
                    Some((at(p, r - k * 2.3 * r), 1.0 - k))
                }
                BallState::Pocketed => None,
            }
        };
        // Soft shadows on the cloth, leaning away from the nearest lamp.
        gl_use_material(&self.shadow_mat);
        let mut shadows = vec![empty_mesh(Some(self.white.clone()))];
        for b in &world.balls {
            let Some((p, fade)) = place(b) else { continue };
            let lx = p.x.clamp(-0.75, 0.75);
            let c = vec3(
                p.x + (p.x - lx) * 0.35 * r,
                p.y + p.y * 0.35 * r - 0.10 * r,
                0.0006,
            );
            let s = r * 1.42;
            let col = Color::new(0.0, 0.0, 0.0, 0.72 * fade);
            push_quad(
                &mut shadows,
                [
                    c + vec3(-s, s, 0.0),
                    c + vec3(s, s, 0.0),
                    c + vec3(s, -s, 0.0),
                    c + vec3(-s, -s, 0.0),
                ],
                UV,
                [col; 4],
            );
        }
        for m in &shadows {
            draw_mesh(m);
        }
        // Balls, far to near so their soft edges blend correctly.
        let basis = cam.basis();
        let dpi = screen_dpi_scale().max(1.0);
        let mut order: Vec<(f32, &physics::Ball, Vec3, f32)> = world
            .balls
            .iter()
            .filter_map(|b| place(b).map(|(p, fade)| ((p - cam.pos).length_squared(), b, p, fade)))
            .collect();
        order.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        gl_use_material(&self.ball3d);
        for (_, b, p, fade) in order {
            let rad_px = (cam.px_per_m(p) * r).max(1.0);
            let scale = (rad_px + 1.5) / rad_px;
            self.set_ball_uniforms(
                &self.ball3d,
                b.id,
                b.rotation_columns(),
                (p.x, p.y),
                0.25 + 0.75 * fade,
                1.0,
                scale,
                1.0 / (rad_px * dpi),
                basis,
            );
            let (rx, ry) = (basis.0 * r * scale, basis.1 * r * scale);
            let mesh = quad_mesh(
                [p - rx - ry, p + rx - ry, p + rx + ry, p - rx + ry],
                UV,
                WHITE,
                Some(self.atlas.texture.clone()),
            );
            draw_mesh(&mesh);
        }
        gl_use_default_material();
        set_default_camera();
    }

    /// Cue stick with its tip `gap` metres behind the cue ball surface,
    /// drawn as a tapered, shaded rod in perspective.
    pub fn draw_cue(&self, cam: &Cam, ball: physics::V2, angle: f64, gap: f64, alpha: f32) {
        let back = vec3(-(angle.cos() as f32), -(angle.sin() as f32), 0.0);
        let wood = self.theme().wood1;
        let butt = Color::new(
            wood[0] * 0.8 + 0.04,
            wood[1] * 0.8 + 0.03,
            wood[2] * 0.8 + 0.03,
            1.0,
        );
        // start, end, half width at start, half width at end, colour
        let parts: [(f32, f32, f32, f32, Color); 8] = [
            (
                0.0,
                0.007,
                0.0056,
                0.0058,
                Color::new(0.22, 0.42, 0.62, 1.0),
            ),
            (
                0.007,
                0.030,
                0.0059,
                0.0061,
                Color::new(0.95, 0.93, 0.87, 1.0),
            ),
            (
                0.030,
                0.72,
                0.0061,
                0.0104,
                Color::new(0.87, 0.73, 0.48, 1.0),
            ),
            (
                0.72,
                0.737,
                0.0106,
                0.0107,
                Color::new(0.80, 0.68, 0.34, 1.0),
            ),
            (0.737, 1.05, 0.0107, 0.0134, butt),
            (
                1.05,
                1.33,
                0.0134,
                0.0147,
                Color::new(0.09, 0.09, 0.10, 1.0),
            ),
            (1.33, 1.455, 0.0147, 0.0155, butt),
            (
                1.455,
                1.475,
                0.0150,
                0.0138,
                Color::new(0.04, 0.04, 0.04, 1.0),
            ),
        ];
        let start = at(ball, R as f32) + back * (R as f32 + 0.003 + gap as f32);
        // The butt is raised a little so the cue clears the rails.
        let axis = |t: f32| start + back * t + vec3(0.0, 0.0, t * 0.085);
        let ground = |t: f32| {
            let a = axis(t);
            vec3(a.x + 0.3 * a.z, a.y - 0.5 * a.z, 0.001)
        };
        let strip = |a3: Vec3, b3: Vec3, wa: f32, wb: f32, scale: f32, shift: f32, c: Color| {
            let (Some(pa), Some(pb)) = (cam.project(a3), cam.project(b3)) else {
                return;
            };
            let dir = (pb - pa).normalize_or_zero();
            let perp = vec2(-dir.y, dir.x);
            let (ha, hb) = (wa * cam.px_per_m(a3), wb * cam.px_per_m(b3));
            let (ca, cb) = (pa + perp * ha * shift, pb + perp * hb * shift);
            let (p1, p2, p3, p4) = (
                ca + perp * ha * scale,
                cb + perp * hb * scale,
                cb - perp * hb * scale,
                ca - perp * ha * scale,
            );
            draw_triangle(p1, p2, p3, c);
            draw_triangle(p1, p3, p4, c);
        };
        let tone = |c: Color, k: f32| {
            Color::new(
                (c.r * k).min(1.0),
                (c.g * k).min(1.0),
                (c.b * k).min(1.0),
                alpha,
            )
        };
        // Long parts are cut into pieces so perspective tapers them properly.
        let mut pieces: Vec<(f32, f32, f32, f32, Color)> = Vec::new();
        for &(a, b, wa, wb, c) in parts.iter() {
            let n = (((b - a) / 0.08).ceil() as usize).max(1);
            for k in 0..n {
                let (t0, t1) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
                pieces.push((
                    a + (b - a) * t0,
                    a + (b - a) * t1,
                    wa + (wb - wa) * t0,
                    wa + (wb - wa) * t1,
                    c,
                ));
            }
        }
        for &(a, b, wa, wb, _) in &pieces {
            strip(
                ground(a),
                ground(b),
                wa,
                wb,
                1.0,
                0.0,
                Color::new(0.0, 0.0, 0.0, 0.26 * alpha),
            );
        }
        for &(a, b, wa, wb, c) in &pieces {
            strip(axis(a), axis(b), wa, wb, 1.0, 0.0, tone(c, 0.55));
            strip(axis(a), axis(b), wa, wb, 0.74, 0.0, tone(c, 0.85));
            strip(axis(a), axis(b), wa, wb, 0.42, 0.12, tone(c, 1.08));
            strip(axis(a), axis(b), wa, wb, 0.12, 0.30, tone(c, 1.45));
        }
    }

    /// Rounded rectangle with an optional border.
    pub fn panel(&self, r: Rect, radius: f32, fill: Color, border: Color, border_w: f32) {
        let m = &self.panel_mat;
        m.set_uniform("u_size", vec2(r.w, r.h));
        m.set_uniform("u_fill", vec4(fill.r, fill.g, fill.b, fill.a));
        m.set_uniform("u_border", vec4(border.r, border.g, border.b, border.a));
        m.set_uniform("u_rb", vec2(radius.min(r.w / 2.0).min(r.h / 2.0), border_w));
        gl_use_material(m);
        draw_texture_ex(
            &self.white,
            r.x,
            r.y,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(r.w, r.h)),
                ..Default::default()
            },
        );
        gl_use_default_material();
    }

    fn font(&self, bold: bool) -> Option<&Font> {
        if bold {
            self.font_bold.as_ref()
        } else {
            self.font.as_ref()
        }
    }

    fn text_params(&self, size: f32, color: Color, bold: bool) -> TextParams<'_> {
        // Rasterise at device resolution so text stays sharp on scaled displays.
        let dpi = screen_dpi_scale().max(1.0);
        TextParams {
            font: self.font(bold),
            font_size: (size * dpi).round().clamp(4.0, 400.0) as u16,
            font_scale: 1.0 / dpi,
            color,
            ..Default::default()
        }
    }

    pub fn text_width(&self, s: &str, size: f32, bold: bool) -> f32 {
        let dpi = screen_dpi_scale().max(1.0);
        measure_text(
            s,
            self.font(bold),
            (size * dpi).round().clamp(4.0, 400.0) as u16,
            1.0 / dpi,
        )
        .width
    }

    /// Text with its left edge at `x` and its vertical centre at `y`.
    pub fn text(&self, s: &str, x: f32, y: f32, size: f32, color: Color, bold: bool) {
        draw_text_ex(
            s,
            x.round(),
            (y + size * 0.34).round(),
            self.text_params(size, color, bold),
        );
    }

    pub fn text_center(&self, s: &str, x: f32, y: f32, size: f32, color: Color, bold: bool) {
        let w = self.text_width(s, size, bold);
        self.text(s, x - w / 2.0, y, size, color, bold);
    }

    pub fn text_right(&self, s: &str, x: f32, y: f32, size: f32, color: Color, bold: bool) {
        let w = self.text_width(s, size, bold);
        self.text(s, x - w, y, size, color, bold);
    }
}
