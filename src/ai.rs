//! Computer opponent.
//!
//! The AI lists shots that look possible on paper, plays each one out in a
//! private copy of the table using the real physics, lets the real referee
//! judge the result and keeps the best. Skill levels differ in how many
//! options they look at and how steady their hand is.

use crate::physics::{HALF_L, HEAD_SPOT, R, Rng, V2, World, path_clear, v2};
use crate::rules::{Choice, Decision, Mode, Outcome, Rules, ShotCall, ShotContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Elite,
}

impl Difficulty {
    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
            Difficulty::Elite => "Elite",
        }
    }
    /// Standard deviation of the aiming error, in radians.
    fn aim_error(self) -> f64 {
        match self {
            Difficulty::Easy => 0.85_f64.to_radians(),
            Difficulty::Medium => 0.3_f64.to_radians(),
            Difficulty::Hard => 0.07_f64.to_radians(),
            Difficulty::Elite => 0.015_f64.to_radians(),
        }
    }
    fn max_sims(self) -> usize {
        match self {
            Difficulty::Easy => 24,
            Difficulty::Medium => 60,
            Difficulty::Hard => 130,
            Difficulty::Elite => 340,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AiShot {
    /// Where to put the cue ball first, when it is in hand.
    pub place: Option<V2>,
    pub angle: f64,
    pub speed: f64,
    pub side: f64,
    pub top: f64,
    pub call: ShotCall,
}

/// What the computer picks when the rules offer it a choice.
pub fn choose(d: &Decision) -> Choice {
    let prefer = [
        Choice::SpotEight,
        Choice::SpotEightBehindLine,
        Choice::RerackAndBreak,
        Choice::BallInHandBehindLine,
        Choice::ShootAfterPush,
        Choice::AcceptTable,
    ];
    prefer
        .into_iter()
        .find(|c| d.options.contains(c))
        .unwrap_or(d.options[0])
}

#[derive(Clone, Copy)]
struct Candidate {
    call: ShotCall,
    cue: V2,
    angle: f64,
    speed: f64,
    top: f64,
    /// Lower is easier.
    difficulty: f64,
}

fn score(me: usize, out: &Outcome, potted: usize) -> f64 {
    if let Some(w) = out.winner {
        return if w == me { 1000.0 } else { -1000.0 };
    }
    if out.foul.is_some() {
        return -100.0;
    }
    if out.next_player == me {
        50.0 + 5.0 * potted as f64
    } else {
        0.0
    }
}

/// Shots that pocket `target` directly, for a cue ball at `cue`.
fn pot_candidates(world: &World, cue: V2, legal: &[u8], out: &mut Vec<Candidate>) {
    for &id in legal {
        let Some(target) = world.ball(id).filter(|b| b.on_table()).map(|b| b.pos) else {
            continue;
        };
        for (pi, pocket) in world.pockets.iter().enumerate() {
            let to_pocket = pocket.aim - target;
            let d_pocket = to_pocket.len();
            let dir_tp = to_pocket.norm();
            let ghost = target - dir_tp * (2.0 * R);
            let to_ghost = ghost - cue;
            let d_cue = to_ghost.len();
            if d_cue < 1e-6 {
                continue;
            }
            let dir_cg = to_ghost.norm();
            let cos_cut = dir_cg.dot(dir_tp);
            if cos_cut < 0.26 {
                continue; // thinner than about 75 degrees
            }
            if !path_clear(world, cue, ghost, &[0, id])
                || !path_clear(world, target, pocket.aim, &[0, id])
            {
                continue;
            }
            let difficulty = (d_cue + 0.2) * (d_pocket + 0.2) / (cos_cut * cos_cut);
            // Enough pace to reach the pocket comfortably, more for thin cuts.
            let base = (0.9 + 1.1 * (d_cue + d_pocket)) / cos_cut.max(0.35);
            for (mult, top) in [
                (1.0, 0.0),
                (1.5, 0.0),
                (1.0, -0.6),
                (1.25, 0.6),
                (0.75, 0.0),
            ] {
                out.push(Candidate {
                    call: ShotCall {
                        ball: Some(id),
                        pocket: Some(pi),
                        ..ShotCall::default()
                    },
                    cue,
                    angle: dir_cg.angle(),
                    speed: (base * mult).clamp(0.6, 6.5),
                    top,
                    difficulty,
                });
            }
        }
    }
}

/// Places worth trying when the cue ball is in hand.
fn placements(world: &World, ctx: &ShotContext, kitchen: bool) -> Vec<V2> {
    let mut spots: Vec<(f64, V2)> = Vec::new();
    for &id in &ctx.legal {
        let Some(target) = world.ball(id).filter(|b| b.on_table()).map(|b| b.pos) else {
            continue;
        };
        for pocket in &world.pockets {
            let dir = (pocket.aim - target).norm();
            if !path_clear(world, target, pocket.aim, &[0, id]) {
                continue;
            }
            for back in [0.3, 0.5] {
                let p = target - dir * (2.0 * R + back);
                if !World::in_bounds(p) || !world.spot_free(p, 0) {
                    continue;
                }
                if kitchen && p.x > HEAD_SPOT.x {
                    continue;
                }
                if !path_clear(world, p, target - dir * (2.0 * R), &[0, id]) {
                    continue;
                }
                spots.push(((pocket.aim - target).len(), p));
            }
        }
    }
    spots.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    spots.into_iter().take(6).map(|s| s.1).collect()
}

/// Balls the next shooter may hit after `out`, on the simulated table.
fn next_legal(rules: &Rules, out: &Outcome, sim: &World) -> Vec<u8> {
    let mut r = rules.clone();
    if let Some(g) = out.assign {
        r.groups[r.current] = Some(g);
        r.groups[1 - r.current] = Some(g.other());
    }
    r.current = out.next_player;
    r.break_shot = false;
    r.ball_in_hand = false;
    r.context(sim).legal
}

/// How easy the easiest direct pot is from `cue` (lower is easier).
fn easiest(world: &World, cue: V2, legal: &[u8]) -> f64 {
    let mut v = Vec::new();
    pot_candidates(world, cue, legal, &mut v);
    v.iter().map(|c| c.difficulty).fold(f64::INFINITY, f64::min)
}

fn simulate(
    world: &World,
    rules: &Rules,
    ctx: &ShotContext,
    c: &mut Candidate,
    diff: Difficulty,
) -> f64 {
    let mut sim = world.clone();
    sim.balls[0].pos = c.cue;
    sim.begin_shot();
    sim.strike(c.angle, c.speed, 0.0, c.top);
    sim.run_to_rest(30.0);
    // A shot found by searching (a bank, kick or combination) is called for
    // the ball and pocket it is known to make.
    if c.call.ball.is_none() && !c.call.safety {
        let made = sim.log.potted.iter().position(|b| ctx.legal.contains(b));
        match made {
            Some(i) => {
                c.call = ShotCall {
                    ball: Some(sim.log.potted[i]),
                    pocket: sim.log.pockets.get(i).copied(),
                    ..ShotCall::default()
                }
            }
            None => c.call.safety = true,
        }
    }
    let out = rules.judge(ctx, &c.call, &sim.log, &sim);
    let mut s = score(rules.current, &out, sim.log.potted.len());
    // Small preference for leaving the cue ball away from the rails.
    if out.winner.is_none() && sim.on_table(0) {
        let p = sim.cue().pos;
        let edge = (HALF_L - p.x.abs()).min(crate::physics::HALF_W - p.y.abs());
        s += edge.min(0.3) * 10.0;
    }
    // Strong players think one shot ahead: keep an easy ball for themselves,
    // leave nothing easy for the opponent.
    let smart = matches!(diff, Difficulty::Hard | Difficulty::Elite);
    if smart
        && out.winner.is_none()
        && out.foul.is_none()
        && out.decision.is_none()
        && sim.on_table(0)
    {
        let e = easiest(&sim, sim.cue().pos, &next_legal(rules, &out, &sim));
        let value = 28.0 / (1.0 + e);
        s += if out.next_player == rules.current {
            value
        } else {
            -value
        };
    }
    s
}

/// Choose a shot for the player whose turn it is.
pub fn plan(world: &World, rules: &Rules, diff: Difficulty, seed: u64) -> AiShot {
    let mut rng = Rng::new(seed);
    let ctx = rules.context(world);
    let cue_now = world.cue().pos;

    // The break: hit the head of the rack hard, slightly off centre.
    if rules.break_shot && rules.mode != Mode::Practice {
        let cue = if rules.ball_in_hand {
            world.find_free_spot(v2(HEAD_SPOT.x, 0.12 * rng.signed()), 0)
        } else {
            cue_now
        };
        let apex = world
            .balls
            .iter()
            .filter(|b| b.id != 0 && b.on_table())
            .min_by(|a, b| a.pos.x.partial_cmp(&b.pos.x).unwrap())
            .map_or(v2(0.6, 0.0), |b| b.pos);
        let angle = (apex - cue).angle() + rng.gauss() * diff.aim_error();
        let speed = match diff {
            Difficulty::Easy => 6.5,
            Difficulty::Medium => 7.8,
            Difficulty::Hard => 8.8,
            Difficulty::Elite => 9.0,
        };
        return AiShot {
            place: Some(cue),
            angle,
            speed,
            side: 0.0,
            top: 0.15,
            call: ShotCall::default(),
        };
    }

    let mut cands: Vec<Candidate> = Vec::new();
    pot_candidates(world, cue_now, &ctx.legal, &mut cands);
    if rules.ball_in_hand {
        for p in placements(world, &ctx, rules.kitchen) {
            pot_candidates(world, p, &ctx.legal, &mut cands);
        }
    }
    cands.sort_by(|a, b| a.difficulty.partial_cmp(&b.difficulty).unwrap());
    if diff == Difficulty::Easy {
        // A beginner only considers plain centre-ball shots.
        cands.retain(|c| c.top == 0.0);
    }
    cands.truncate(diff.max_sims());

    let mut best: Option<(f64, Candidate)> = None;
    let caution = if diff == Difficulty::Elite { 0.4 } else { 2.0 };
    for c in cands.iter_mut() {
        let s = simulate(world, rules, &ctx, c, diff) - c.difficulty * caution;
        if best.map_or(true, |(bs, _)| s > bs) {
            best = Some((s, *c));
        }
    }

    // Nothing pots: look for any legal contact, including kicks off the rails.
    if best.map_or(true, |(s, _)| s < 25.0) {
        let mut safeties: Vec<Candidate> = Vec::new();
        let safety = ShotCall {
            safety: true,
            ..ShotCall::default()
        };
        for &id in &ctx.legal {
            if let Some(b) = world.ball(id).filter(|b| b.on_table()) {
                let base = (b.pos - cue_now).angle();
                for off in [0.0, 0.012, -0.012, 0.03, -0.03] {
                    for speed in [1.2, 2.2] {
                        safeties.push(Candidate {
                            call: safety,
                            cue: cue_now,
                            angle: base + off,
                            speed,
                            top: 0.0,
                            difficulty: 0.0,
                        });
                    }
                }
            }
        }
        // Sweep every direction for banks, kicks and combinations.
        let (sweeps, speeds): (usize, &[f64]) = match diff {
            Difficulty::Easy => (16, &[2.6]),
            Difficulty::Medium => (48, &[2.6]),
            Difficulty::Hard => (90, &[2.6]),
            Difficulty::Elite => (180, &[1.7, 3.0]),
        };
        let search = if diff == Difficulty::Easy {
            safety
        } else {
            ShotCall::default()
        };
        for k in 0..sweeps {
            let angle = k as f64 / sweeps as f64 * std::f64::consts::TAU + rng.f64() * 0.01;
            for &speed in speeds {
                safeties.push(Candidate {
                    call: search,
                    cue: cue_now,
                    angle,
                    speed,
                    top: 0.0,
                    difficulty: 0.0,
                });
            }
        }
        for c in safeties.iter_mut() {
            let s = simulate(world, rules, &ctx, c, diff);
            if best.map_or(true, |(bs, _)| s > bs) {
                best = Some((s, *c));
            }
        }
    }

    let c = best.map(|b| b.1).unwrap_or(Candidate {
        call: ShotCall {
            safety: true,
            ..ShotCall::default()
        },
        cue: cue_now,
        angle: rng.f64() * std::f64::consts::TAU,
        speed: 2.0,
        top: 0.0,
        difficulty: 0.0,
    });
    AiShot {
        place: if (c.cue - cue_now).len2() > 1e-12 {
            Some(c.cue)
        } else {
            None
        },
        angle: c.angle + rng.gauss() * diff.aim_error(),
        speed: (c.speed * (1.0 + rng.gauss() * 0.02)).clamp(0.4, 9.0),
        side: 0.0,
        top: c.top,
        call: c.call,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{Ball, rack_eight_ball, rack_nine_ball};

    fn names() -> [String; 2] {
        ["A".to_string(), "B".to_string()]
    }

    /// Resolve decisions and re-racks the way the game loop does.
    fn settle(r: &mut Rules, w: &mut World, rng: &mut Rng) {
        if let Some(d) = r.pending.clone() {
            r.choose(w, choose(&d));
        }
        if let Some(b) = r.rerack {
            if r.mode == Mode::NineBall {
                rack_nine_ball(w, rng);
            } else {
                rack_eight_ball(w, rng);
            }
            r.reset_rack(b);
        }
    }

    #[test]
    fn hard_ai_pots_a_simple_shot() {
        let mut w = World::new();
        w.balls.push(Ball::new(0, v2(0.0, 0.0)));
        w.balls.push(Ball::new(1, v2(0.7, 0.35)));
        w.balls.push(Ball::new(8, v2(-0.8, -0.3)));
        w.balls.push(Ball::new(12, v2(-0.5, 0.4)));
        let mut r = Rules::new(Mode::EightBall, names(), 0);
        r.break_shot = false;
        r.ball_in_hand = false;
        r.kitchen = false;
        r.groups = [
            Some(crate::rules::Group::Solids),
            Some(crate::rules::Group::Stripes),
        ];
        let shot = plan(&w, &r, Difficulty::Hard, 11);
        let ctx = r.context(&w);
        w.begin_shot();
        w.strike(shot.angle, shot.speed, shot.side, shot.top);
        w.run_to_rest(30.0);
        let out = r.judge(&ctx, &shot.call, &w.log, &w);
        assert_eq!(out.foul, None);
        assert_eq!(w.log.potted, vec![1]);
    }

    /// Whole matches, computer against computer, must always reach a winner.
    #[test]
    fn full_games_finish() {
        for (seed, mode) in [
            (21u64, Mode::EightBall),
            (22, Mode::NineBall),
            (23, Mode::EightBall),
            (24, Mode::NineBall),
        ] {
            let mut rng = Rng::new(seed);
            let mut w = World::new();
            if mode == Mode::NineBall {
                rack_nine_ball(&mut w, &mut rng);
            } else {
                rack_eight_ball(&mut w, &mut rng);
            }
            let mut r = Rules::new(mode, names(), 0);
            let mut turns = 0;
            while r.winner.is_none() {
                turns += 1;
                assert!(turns < 400, "seed {seed}: game did not finish");
                let shot = plan(&w, &r, Difficulty::Easy, rng.next_u64());
                if let Some(p) = shot.place {
                    assert!(
                        World::in_bounds(p) && w.spot_free(p, 0),
                        "seed {seed}: bad placement"
                    );
                    w.balls[0].pos = p;
                }
                let ctx = r.context(&w);
                assert!(!ctx.legal.is_empty(), "seed {seed}: nothing legal to hit");
                w.begin_shot();
                w.strike(shot.angle, shot.speed, shot.side, shot.top);
                assert!(
                    w.run_to_rest(90.0) < 90.0,
                    "seed {seed}: shot never settled"
                );
                let out = r.judge(&ctx, &shot.call, &w.log, &w);
                r.apply(&mut w, &out);
                settle(&mut r, &mut w, &mut rng);
                assert!(
                    w.on_table(0),
                    "seed {seed}: cue ball missing after the shot"
                );
            }
        }
    }

    /// The top level should clearly outplay the beginner level.
    #[test]
    fn elite_beats_easy() {
        let mut elite_wins = 0;
        for seed in 0..4u64 {
            let mut rng = Rng::new(900 + seed);
            let mut w = World::new();
            rack_eight_ball(&mut w, &mut rng);
            // Alternate who breaks; player 0 is Elite.
            let mut r = Rules::new(Mode::EightBall, names(), (seed % 2) as usize);
            let mut turns = 0;
            while r.winner.is_none() && turns < 300 {
                turns += 1;
                let diff = if r.current == 0 {
                    Difficulty::Elite
                } else {
                    Difficulty::Easy
                };
                let shot = plan(&w, &r, diff, rng.next_u64());
                if let Some(p) = shot.place {
                    w.balls[0].pos = p;
                }
                let ctx = r.context(&w);
                w.begin_shot();
                w.strike(shot.angle, shot.speed, shot.side, shot.top);
                w.run_to_rest(90.0);
                let out = r.judge(&ctx, &shot.call, &w.log, &w);
                r.apply(&mut w, &out);
                settle(&mut r, &mut w, &mut rng);
            }
            if r.winner == Some(0) {
                elite_wins += 1;
            }
        }
        assert!(
            elite_wins >= 3,
            "Elite won only {elite_wins} of 4 against Easy"
        );
    }

    /// Prints how long the planner takes; run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn planning_time() {
        let mut rng = Rng::new(77);
        let mut w = World::new();
        rack_eight_ball(&mut w, &mut rng);
        let mut r = Rules::new(Mode::EightBall, names(), 0);
        w.begin_shot();
        w.strike(0.01, 8.6, 0.0, 0.1);
        w.run_to_rest(60.0);
        let ctx = r.context(&w);
        let out = r.judge(&ctx, &ShotCall::default(), &w.log, &w);
        r.apply(&mut w, &out);
        settle(&mut r, &mut w, &mut rng);
        for diff in [
            Difficulty::Easy,
            Difficulty::Medium,
            Difficulty::Hard,
            Difficulty::Elite,
        ] {
            for hand in [false, true] {
                r.ball_in_hand = hand;
                let t = std::time::Instant::now();
                plan(&w, &r, diff, 5);
                println!("{:?} ball_in_hand={hand}: {:?}", diff, t.elapsed());
            }
        }
    }

    #[test]
    fn ai_always_returns_a_usable_shot() {
        for (seed, nine) in [(1u64, false), (2, true), (3, false), (4, true)] {
            let mut rng = Rng::new(seed);
            let mut w = World::new();
            let mode = if nine {
                Mode::NineBall
            } else {
                Mode::EightBall
            };
            if nine {
                rack_nine_ball(&mut w, &mut rng);
            } else {
                rack_eight_ball(&mut w, &mut rng);
            }
            let mut r = Rules::new(mode, names(), 0);
            // Play several AI turns in a row and check nothing degenerate comes out.
            for turn in 0..6 {
                if r.winner.is_some() {
                    break;
                }
                let shot = plan(&w, &r, Difficulty::Medium, seed * 100 + turn);
                assert!(shot.angle.is_finite() && shot.speed > 0.0 && shot.speed <= 9.0);
                if let Some(p) = shot.place {
                    assert!(World::in_bounds(p) && w.spot_free(p, 0));
                    w.balls[0].pos = p;
                }
                let ctx = r.context(&w);
                w.begin_shot();
                w.strike(shot.angle, shot.speed, shot.side, shot.top);
                assert!(w.run_to_rest(60.0) < 60.0);
                let out = r.judge(&ctx, &shot.call, &w.log, &w);
                r.apply(&mut w, &out);
                settle(&mut r, &mut w, &mut rng);
            }
        }
    }
}
