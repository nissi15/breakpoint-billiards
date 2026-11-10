//! Game rules: 8-ball and 9-ball following the WPA Rules of Play
//! (effective 2026-10-21), plus free practice.
//!
//! Judging a shot is a pure function of the situation before the shot, the
//! shooter's call and the physics log afterwards, so the AI scores simulated
//! shots with the exact code that referees the real game.

use crate::physics::{BallState, HEAD_SPOT, ShotLog, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    EightBall,
    NineBall,
    Practice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Solids,
    Stripes,
}

impl Group {
    pub fn other(self) -> Group {
        match self {
            Group::Solids => Group::Stripes,
            Group::Stripes => Group::Solids,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Group::Solids => "Solids",
            Group::Stripes => "Stripes",
        }
    }
    pub fn balls(self) -> std::ops::RangeInclusive<u8> {
        match self {
            Group::Solids => 1..=7,
            Group::Stripes => 9..=15,
        }
    }
}

pub fn group_of(id: u8) -> Option<Group> {
    match id {
        1..=7 => Some(Group::Solids),
        9..=15 => Some(Group::Stripes),
        _ => None,
    }
}

/// What the shooter announced before the shot (WPA 1.7, 5.4).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ShotCall {
    pub ball: Option<u8>,
    pub pocket: Option<usize>,
    pub safety: bool,
    pub push_out: bool,
}

/// An option a player picks after certain shots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    AcceptTable,
    RerackAndBreak,
    RerackOpponentBreaks,
    SpotEight,
    Rebreak,
    SpotEightBehindLine,
    BallInHandBehindLine,
    ShootAfterPush,
    PassAfterPush,
}

impl Choice {
    pub const ALL: [Choice; 9] = [
        Choice::AcceptTable,
        Choice::RerackAndBreak,
        Choice::RerackOpponentBreaks,
        Choice::SpotEight,
        Choice::Rebreak,
        Choice::SpotEightBehindLine,
        Choice::BallInHandBehindLine,
        Choice::ShootAfterPush,
        Choice::PassAfterPush,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Choice::AcceptTable => "Play the table as it lies",
            Choice::RerackAndBreak => "Re-rack and break yourself",
            Choice::RerackOpponentBreaks => "Re-rack, opponent breaks again",
            Choice::SpotEight => "Spot the 8 and keep shooting",
            Choice::Rebreak => "Re-rack and break again",
            Choice::SpotEightBehindLine => "Spot the 8, ball in hand behind the line",
            Choice::BallInHandBehindLine => "Ball in hand behind the line",
            Choice::ShootAfterPush => "Shoot from here",
            Choice::PassAfterPush => "Pass the shot back",
        }
    }
    pub fn index(self) -> usize {
        Choice::ALL.iter().position(|&c| c == self).unwrap()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// Player who chooses.
    pub by: usize,
    pub title: String,
    pub options: Vec<Choice>,
}

/// The situation at the moment the cue ball is struck.
#[derive(Clone, Debug, PartialEq)]
pub struct ShotContext {
    /// Balls the shooter may legally hit first.
    pub legal: Vec<u8>,
    /// 8-ball: the shooter may shoot the 8 to win.
    pub on_eight: bool,
    pub break_shot: bool,
    /// The cue ball was in hand behind the head string (WPA 3.11 applies).
    pub kitchen: bool,
    pub can_push: bool,
}

/// The referee's decision after a shot.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Outcome {
    pub foul: Option<&'static str>,
    pub next_player: usize,
    pub ball_in_hand: bool,
    pub kitchen: bool,
    pub winner: Option<usize>,
    /// Group claimed by the shooter with this shot.
    pub assign: Option<Group>,
    /// Object balls that go back on the table.
    pub respot: Vec<u8>,
    pub decision: Option<Decision>,
    /// Practice only: every ball is down, rack again.
    pub rerack: bool,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Rules {
    pub mode: Mode,
    pub names: [String; 2],
    /// Player to shoot, 0 or 1.
    pub current: usize,
    pub breaker: usize,
    pub groups: [Option<Group>; 2],
    pub break_shot: bool,
    pub ball_in_hand: bool,
    /// Ball in hand is limited to behind the head string.
    pub kitchen: bool,
    pub winner: Option<usize>,
    /// 9-ball: consecutive fouls by each player.
    pub fouls: [u8; 2],
    /// 9-ball: the next shot may be a push-out.
    pub push_available: bool,
    pub pending: Option<Decision>,
    /// Set when the balls must be racked again; holds who breaks.
    pub rerack: Option<usize>,
    /// Innings in a row with no ball pocketed and no foul (stalemate watch).
    pub quiet: u32,
}

/// Innings without progress before the referee declares a stalemate (WPA 1.13).
pub const STALEMATE_INNINGS: u32 = 30;

impl Rules {
    pub fn new(mode: Mode, names: [String; 2], first: usize) -> Rules {
        Rules {
            mode,
            names,
            current: first,
            breaker: first,
            groups: [None, None],
            break_shot: true,
            ball_in_hand: true,
            kitchen: true,
            winner: None,
            fouls: [0, 0],
            push_available: false,
            pending: None,
            rerack: None,
            quiet: 0,
        }
    }

    /// Start a fresh rack with `breaker` to break (after a re-rack decision).
    pub fn reset_rack(&mut self, breaker: usize) {
        let names = self.names.clone();
        *self = Rules::new(self.mode, names, breaker);
    }

    pub fn remaining(world: &World, group: Group) -> usize {
        group.balls().filter(|&id| world.on_table(id)).count()
    }

    pub fn lowest_ball(world: &World) -> Option<u8> {
        world
            .balls
            .iter()
            .filter(|b| b.id != 0 && b.on_table())
            .map(|b| b.id)
            .min()
    }

    pub fn context(&self, world: &World) -> ShotContext {
        let on_table = |id: u8| world.on_table(id);
        let all: Vec<u8> = (1..=15).filter(|&i| on_table(i)).collect();
        let mut on_eight = false;
        let legal = match self.mode {
            Mode::Practice => all,
            Mode::NineBall => Self::lowest_ball(world).into_iter().collect(),
            Mode::EightBall => {
                if self.break_shot {
                    all
                } else if let Some(g) = self.groups[self.current] {
                    let own: Vec<u8> = g.balls().filter(|&i| on_table(i)).collect();
                    if own.is_empty() {
                        on_eight = true;
                        vec![8]
                    } else {
                        own
                    }
                } else {
                    // Open table: anything but the 8, unless a whole group is
                    // already down, in which case the shooter may claim it.
                    let cleared = Self::remaining(world, Group::Solids) == 0
                        || Self::remaining(world, Group::Stripes) == 0;
                    on_eight = cleared;
                    all.into_iter().filter(|&i| i != 8 || cleared).collect()
                }
            }
        };
        ShotContext {
            legal,
            on_eight,
            break_shot: self.break_shot,
            kitchen: self.kitchen && self.ball_in_hand && self.mode != Mode::Practice,
            can_push: self.mode == Mode::NineBall && self.push_available,
        }
    }

    /// Decide what a finished shot means. Does not change any state.
    pub fn judge(
        &self,
        ctx: &ShotContext,
        call: &ShotCall,
        log: &ShotLog,
        world: &World,
    ) -> Outcome {
        let me = self.current;
        let opp = 1 - me;
        let mut out = Outcome {
            next_player: me,
            ..Outcome::default()
        };

        if self.mode == Mode::Practice {
            out.ball_in_hand = log.cue_potted;
            let left = world
                .balls
                .iter()
                .filter(|b| b.id != 0 && b.state == BallState::OnTable)
                .count();
            if left == 0 {
                out.rerack = true;
                out.message = "Table cleared - racking again".into();
            } else if log.cue_potted {
                out.message = "Scratch - cue ball in hand".into();
            } else if !log.potted.is_empty() {
                out.message = if log.potted.len() == 1 {
                    "Nice shot".into()
                } else {
                    format!("{} balls down", log.potted.len())
                };
            }
            return out;
        }

        let push = call.push_out && ctx.can_push;
        let eight_ball = self.mode == Mode::EightBall;
        let foul = if log.cue_potted {
            Some("Scratch")
        } else if push {
            None
        } else if log.first_hit.is_none() {
            Some("No ball hit")
        } else if !ctx.legal.contains(&log.first_hit.unwrap()) {
            Some("Wrong ball first")
        } else if eight_ball
            && ctx.kitchen
            && !ctx.break_shot
            && log.first_hit_in_kitchen
            && !log.crossed_head_string
        {
            Some("Bad play from behind the head string")
        } else if !ctx.break_shot && log.potted.is_empty() && !log.cushion_after_contact {
            Some("No rail after contact")
        } else if !eight_ball && ctx.break_shot && log.potted.is_empty() && log.balls_to_cushion < 4
        {
            Some("Illegal break - four balls must reach a rail")
        } else {
            None
        };
        out.foul = foul;
        let name = |p: usize| self.names[p].as_str();
        let decide =
            |by: usize, title: String, options: Vec<Choice>| Some(Decision { by, title, options });

        if eight_ball {
            let eight = log.potted.contains(&8);
            if ctx.break_shot {
                let others = log.potted.iter().any(|&b| b != 8);
                if eight && foul.is_some() {
                    out.next_player = opp;
                    out.message = format!("8-ball down on a foul break - {} decides", name(opp));
                    out.decision = decide(
                        opp,
                        "The 8 went down on a foul break".into(),
                        vec![Choice::SpotEightBehindLine, Choice::Rebreak],
                    );
                } else if eight {
                    out.message = format!("8-ball down on the break - {} decides", name(me));
                    out.decision = decide(
                        me,
                        "You pocketed the 8 on the break".into(),
                        vec![Choice::SpotEight, Choice::Rebreak],
                    );
                } else if let Some(f) = foul {
                    out.next_player = opp;
                    out.ball_in_hand = true;
                    out.kitchen = true;
                    if log.cue_potted {
                        out.message = format!(
                            "{f} on the break - ball in hand behind the line for {}",
                            name(opp)
                        );
                    } else {
                        out.ball_in_hand = false;
                        out.kitchen = false;
                        out.message = format!("{f} on the break - {} decides", name(opp));
                        out.decision = decide(
                            opp,
                            format!("Foul on the break: {f}"),
                            vec![Choice::AcceptTable, Choice::BallInHandBehindLine],
                        );
                    }
                } else if others {
                    out.message = format!("{} keeps the table - still open", name(me));
                } else if log.balls_to_cushion < 4 {
                    out.next_player = opp;
                    out.message = format!("Illegal break - {} decides", name(opp));
                    out.decision = decide(
                        opp,
                        "Illegal break: fewer than four balls reached a rail".into(),
                        vec![
                            Choice::AcceptTable,
                            Choice::RerackAndBreak,
                            Choice::RerackOpponentBreaks,
                        ],
                    );
                } else {
                    out.next_player = opp;
                    out.message = format!("{} to shoot - table is open", name(opp));
                }
                return out;
            }
            let pocket_of = |ball: u8| {
                log.potted
                    .iter()
                    .position(|&b| b == ball)
                    .map(|i| log.pockets.get(i).copied())
            };
            if eight {
                let called = !call.safety
                    && call.ball == Some(8)
                    && call.pocket.is_some()
                    && pocket_of(8) == Some(call.pocket);
                if foul.is_none() && ctx.on_eight && called {
                    out.winner = Some(me);
                    out.message = format!("{} sinks the 8 and wins", name(me));
                } else {
                    out.winner = Some(opp);
                    out.message = if foul.is_some() {
                        format!("8-ball pocketed on a foul - {} wins", name(opp))
                    } else if !ctx.on_eight {
                        format!("8-ball down early - {} wins", name(opp))
                    } else {
                        format!("8-ball in an uncalled pocket - {} wins", name(opp))
                    };
                }
                return out;
            }
            if let Some(f) = foul {
                out.next_player = opp;
                out.ball_in_hand = true;
                out.message = format!("{f} - ball in hand for {}", name(opp));
                return out;
            }
            // A called ball counts only if it is the shooter's and drops where called.
            let mine = self.groups[me];
            let called_ok = match (call.safety, call.ball, call.pocket) {
                (false, Some(b), Some(p)) => {
                    b != 8
                        && pocket_of(b) == Some(Some(p))
                        && (mine.is_none() || group_of(b) == mine)
                }
                _ => false,
            };
            if called_ok {
                if mine.is_none() {
                    let g = group_of(call.ball.unwrap()).unwrap();
                    out.assign = Some(g);
                    out.message = format!("{} takes {}", name(me), g.name());
                } else if Self::remaining(world, mine.unwrap()) == 0 {
                    out.message = format!("{} is on the 8", name(me));
                } else {
                    out.message = format!("{} shoots again", name(me));
                }
            } else {
                out.next_player = opp;
                out.message = if call.safety {
                    format!("Safety - {} to shoot", name(opp))
                } else if !log.potted.is_empty() {
                    format!("Not the called shot - {} to shoot", name(opp))
                } else {
                    format!("{} to shoot", name(opp))
                };
            }
            return out;
        }

        // 9-ball.
        let nine = log.potted.contains(&9);
        if nine && foul.is_none() && !push {
            out.winner = Some(me);
            out.message = format!("{} sinks the 9 and wins", name(me));
            return out;
        }
        if nine {
            out.respot.push(9);
        }
        if let Some(f) = foul {
            if self.fouls[me] >= 2 {
                out.winner = Some(opp);
                out.message = format!("Three consecutive fouls - {} wins", name(opp));
                return out;
            }
            out.next_player = opp;
            out.ball_in_hand = true;
            out.message = if self.fouls[me] == 1 {
                format!("{f} - {} is on two fouls", name(me))
            } else {
                format!("{f} - ball in hand for {}", name(opp))
            };
        } else if push {
            out.next_player = opp;
            out.message = format!("Push out - {} decides", name(opp));
            out.decision = decide(
                opp,
                format!("{} played a push out", name(me)),
                vec![Choice::ShootAfterPush, Choice::PassAfterPush],
            );
        } else if log.potted.is_empty() {
            out.next_player = opp;
            out.message = format!("{} to shoot", name(opp));
        } else {
            out.message = format!("{} shoots again", name(me));
        }
        out
    }

    /// WPA 1.6: with ball in hand behind the head string and every legal
    /// ball also behind it, the nearest legal ball is spotted.
    fn kitchen_relief(&mut self, world: &mut World) {
        if !(self.kitchen && self.ball_in_hand) || self.break_shot || self.mode != Mode::EightBall {
            return;
        }
        let legal = self.context(world).legal;
        let pos = |id: u8| world.ball(id).map_or(0.0, |b| b.pos.x);
        if !legal.is_empty() && legal.iter().all(|&id| pos(id) < HEAD_SPOT.x) {
            let nearest = *legal
                .iter()
                .max_by(|a, b| pos(**a).partial_cmp(&pos(**b)).unwrap())
                .unwrap();
            world.spot_ball(nearest);
        }
    }

    /// Apply a decision to the rules state and put balls back where needed.
    pub fn apply(&mut self, world: &mut World, out: &Outcome) {
        let me = self.current;
        let was_break = self.break_shot;
        if let Some(g) = out.assign {
            self.groups[me] = Some(g);
            self.groups[1 - me] = Some(g.other());
        }
        for &id in &out.respot {
            world.spot_ball(id);
        }
        if !world.on_table(0) {
            world.place_ball(0, HEAD_SPOT);
        }
        if self.mode == Mode::NineBall {
            self.fouls[me] = if out.foul.is_some() {
                self.fouls[me] + 1
            } else {
                0
            };
            self.push_available = was_break && out.foul.is_none() && out.winner.is_none();
        }
        let progress = out.foul.is_some()
            || out.assign.is_some()
            || out.next_player == me
            || out.decision.is_some();
        self.quiet = if progress { 0 } else { self.quiet + 1 };
        self.break_shot = false;
        self.current = out.next_player;
        self.ball_in_hand = out.ball_in_hand;
        self.kitchen = out.kitchen;
        self.winner = out.winner;
        self.pending = out.decision.clone();
        if self.winner.is_none()
            && self.pending.is_none()
            && self.mode != Mode::Practice
            && self.quiet >= STALEMATE_INNINGS
        {
            self.rerack = Some(self.breaker);
        }
        self.kitchen_relief(world);
    }

    /// Resolve the pending decision.
    pub fn choose(&mut self, world: &mut World, choice: Choice) {
        let Some(d) = self.pending.take() else { return };
        let choice = if d.options.contains(&choice) {
            choice
        } else {
            d.options[0]
        };
        let by = d.by;
        self.current = by;
        self.ball_in_hand = false;
        self.kitchen = false;
        match choice {
            Choice::AcceptTable | Choice::ShootAfterPush => {}
            Choice::PassAfterPush => self.current = 1 - by,
            Choice::RerackAndBreak | Choice::Rebreak => self.rerack = Some(by),
            Choice::RerackOpponentBreaks => self.rerack = Some(1 - by),
            Choice::SpotEight => world.spot_ball(8),
            Choice::SpotEightBehindLine => {
                world.spot_ball(8);
                self.ball_in_hand = true;
                self.kitchen = true;
            }
            Choice::BallInHandBehindLine => {
                self.ball_in_hand = true;
                self.kitchen = true;
            }
        }
        if self.ball_in_hand && !world.spot_free(world.cue().pos, 0)
            || (self.kitchen && world.cue().pos.x > HEAD_SPOT.x)
        {
            world.place_ball(0, HEAD_SPOT);
        }
        self.kitchen_relief(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{FOOT_SPOT, Rng, rack_eight_ball, rack_nine_ball, v2};

    fn names() -> [String; 2] {
        ["A".to_string(), "B".to_string()]
    }

    fn eight_ball() -> (Rules, World) {
        let mut w = World::new();
        rack_eight_ball(&mut w, &mut Rng::new(3));
        (Rules::new(Mode::EightBall, names(), 0), w)
    }

    fn nine_ball() -> (Rules, World) {
        let mut w = World::new();
        rack_nine_ball(&mut w, &mut Rng::new(5));
        (Rules::new(Mode::NineBall, names(), 0), w)
    }

    fn pot(w: &mut World, ids: &[u8]) {
        for &id in ids {
            w.ball_mut(id).unwrap().state = BallState::Pocketed;
        }
    }

    /// A log where every potted ball went into pocket 2 and plenty reached the rails.
    fn log(first: Option<u8>, potted: &[u8], scratch: bool) -> ShotLog {
        ShotLog {
            first_hit: first,
            potted: potted.to_vec(),
            pockets: vec![2; potted.len()],
            cue_potted: scratch,
            cushion_after_contact: true,
            balls_to_cushion: 6,
            ..ShotLog::default()
        }
    }

    fn call(ball: u8, pocket: usize) -> ShotCall {
        ShotCall {
            ball: Some(ball),
            pocket: Some(pocket),
            ..ShotCall::default()
        }
    }

    const NONE: ShotCall = ShotCall {
        ball: None,
        pocket: None,
        safety: false,
        push_out: false,
    };

    #[test]
    fn dry_legal_break_passes_the_table() {
        let (mut r, mut w) = eight_ball();
        let ctx = r.context(&w);
        assert!(ctx.kitchen);
        let out = r.judge(&ctx, &NONE, &log(Some(1), &[], false), &w);
        assert_eq!(
            (out.foul, out.next_player, out.decision.is_none()),
            (None, 1, true)
        );
        r.apply(&mut w, &out);
        assert!(!r.break_shot && r.groups == [None, None]);
    }

    #[test]
    fn illegal_break_gives_the_opponent_three_options() {
        let (mut r, mut w) = eight_ball();
        let ctx = r.context(&w);
        let mut l = log(Some(1), &[], false);
        l.balls_to_cushion = 3;
        let out = r.judge(&ctx, &NONE, &l, &w);
        let d = out.decision.clone().unwrap();
        assert_eq!(d.by, 1);
        assert_eq!(
            d.options,
            vec![
                Choice::AcceptTable,
                Choice::RerackAndBreak,
                Choice::RerackOpponentBreaks
            ]
        );
        r.apply(&mut w, &out);
        r.choose(&mut w, Choice::RerackOpponentBreaks);
        assert_eq!(r.rerack, Some(0));
    }

    #[test]
    fn break_scratch_is_ball_in_hand_behind_the_line() {
        let (mut r, mut w) = eight_ball();
        let ctx = r.context(&w);
        w.balls[0].state = BallState::Pocketed;
        let out = r.judge(&ctx, &NONE, &log(Some(1), &[3], true), &w);
        assert_eq!(out.foul, Some("Scratch"));
        assert!(out.ball_in_hand && out.kitchen && out.next_player == 1);
        r.apply(&mut w, &out);
        assert!(w.on_table(0) && r.kitchen && r.context(&w).kitchen);
    }

    #[test]
    fn eight_on_the_break_offers_spot_or_rebreak() {
        let (mut r, mut w) = eight_ball();
        let ctx = r.context(&w);
        pot(&mut w, &[8]);
        let out = r.judge(&ctx, &NONE, &log(Some(1), &[8], false), &w);
        assert_eq!(out.winner, None);
        assert_eq!(
            out.decision.as_ref().unwrap().options,
            vec![Choice::SpotEight, Choice::Rebreak]
        );
        r.apply(&mut w, &out);
        r.choose(&mut w, Choice::SpotEight);
        assert!(w.on_table(8) && r.current == 0 && r.pending.is_none());
        // With a scratch the opponent chooses instead.
        let (r2, w2) = eight_ball();
        let out = r2.judge(&r2.context(&w2), &NONE, &log(Some(1), &[8], true), &w2);
        let d = out.decision.unwrap();
        assert_eq!(
            (d.by, d.options),
            (1, vec![Choice::SpotEightBehindLine, Choice::Rebreak])
        );
    }

    #[test]
    fn groups_need_a_called_ball_in_the_called_pocket() {
        let (mut r, mut w) = eight_ball();
        r.break_shot = false;
        r.ball_in_hand = false;
        let ctx = r.context(&w);
        assert!(!ctx.legal.contains(&8));
        pot(&mut w, &[11]);
        let l = log(Some(11), &[11], false);
        // Wrong pocket: ball stays down, table stays open, turn passes.
        let out = r.judge(&ctx, &call(11, 4), &l, &w);
        assert_eq!((out.assign, out.next_player, out.foul), (None, 1, None));
        // Uncalled slop does not count either.
        assert_eq!(r.judge(&ctx, &NONE, &l, &w).next_player, 1);
        // Called correctly: the group is theirs.
        let out = r.judge(&ctx, &call(11, 2), &l, &w);
        assert_eq!((out.assign, out.next_player), (Some(Group::Stripes), 0));
        r.apply(&mut w, &out);
        assert_eq!(r.groups, [Some(Group::Stripes), Some(Group::Solids)]);
    }

    #[test]
    fn a_called_safety_passes_the_turn() {
        let (mut r, w) = eight_ball();
        r.break_shot = false;
        r.groups = [Some(Group::Solids), Some(Group::Stripes)];
        let ctx = r.context(&w);
        let c = ShotCall {
            ball: Some(3),
            pocket: Some(2),
            safety: true,
            push_out: false,
        };
        let out = r.judge(&ctx, &c, &log(Some(3), &[3], false), &w);
        assert_eq!((out.foul, out.next_player), (None, 1));
    }

    #[test]
    fn standard_fouls() {
        let (mut r, w) = eight_ball();
        r.break_shot = false;
        r.ball_in_hand = false;
        r.groups = [Some(Group::Solids), Some(Group::Stripes)];
        let ctx = r.context(&w);
        assert_eq!(
            r.judge(&ctx, &NONE, &log(Some(12), &[], false), &w).foul,
            Some("Wrong ball first")
        );
        assert_eq!(
            r.judge(&ctx, &NONE, &log(None, &[], false), &w).foul,
            Some("No ball hit")
        );
        let mut l = log(Some(2), &[], false);
        l.cushion_after_contact = false;
        let out = r.judge(&ctx, &NONE, &l, &w);
        assert_eq!(out.foul, Some("No rail after contact"));
        assert!(out.ball_in_hand && !out.kitchen);
    }

    #[test]
    fn bad_play_from_behind_the_head_string() {
        let (mut r, w) = eight_ball();
        r.break_shot = false;
        r.ball_in_hand = true;
        r.kitchen = true;
        let ctx = r.context(&w);
        let mut l = log(Some(2), &[], false);
        l.first_hit_in_kitchen = true;
        assert_eq!(
            r.judge(&ctx, &NONE, &l, &w).foul,
            Some("Bad play from behind the head string")
        );
        l.crossed_head_string = true;
        assert_eq!(r.judge(&ctx, &NONE, &l, &w).foul, None);
    }

    #[test]
    fn all_legal_balls_behind_the_line_get_one_spotted() {
        let (mut r, mut w) = eight_ball();
        r.break_shot = false;
        r.groups = [Some(Group::Solids), Some(Group::Stripes)];
        pot(&mut w, &[2, 3, 4, 5, 6, 7]);
        w.ball_mut(1).unwrap().pos = v2(-1.0, 0.2);
        r.pending = Some(Decision {
            by: 0,
            title: String::new(),
            options: vec![Choice::BallInHandBehindLine],
        });
        r.choose(&mut w, Choice::BallInHandBehindLine);
        assert!(r.kitchen && r.ball_in_hand);
        assert!(
            w.ball(1).unwrap().pos.x >= FOOT_SPOT.x - 1e-9,
            "the 1 should have been spotted"
        );
    }

    #[test]
    fn every_way_to_win_or_lose_on_the_eight() {
        let (mut r, mut w) = eight_ball();
        r.break_shot = false;
        r.ball_in_hand = false;
        r.groups = [Some(Group::Solids), Some(Group::Stripes)];
        let ctx = r.context(&w);
        let l = log(Some(1), &[8], false);
        assert_eq!(
            r.judge(&ctx, &call(1, 2), &l, &w).winner,
            Some(1),
            "early 8 loses"
        );
        pot(&mut w, &[1, 2, 3, 4, 5, 6, 7]);
        let ctx = r.context(&w);
        assert!(ctx.on_eight && ctx.legal == vec![8]);
        let l = log(Some(8), &[8], false);
        assert_eq!(
            r.judge(&ctx, &call(8, 2), &l, &w).winner,
            Some(0),
            "called 8 wins"
        );
        assert_eq!(
            r.judge(&ctx, &call(8, 5), &l, &w).winner,
            Some(1),
            "uncalled pocket loses"
        );
        assert_eq!(
            r.judge(&ctx, &NONE, &l, &w).winner,
            Some(1),
            "no call loses"
        );
        assert_eq!(
            r.judge(&ctx, &call(8, 2), &log(Some(8), &[8], true), &w)
                .winner,
            Some(1),
            "8 with a scratch loses"
        );
        // A scratch on the 8 without pocketing it is only a foul.
        let out = r.judge(&ctx, &call(8, 2), &log(Some(8), &[], true), &w);
        assert_eq!((out.winner, out.foul), (None, Some("Scratch")));
    }

    #[test]
    fn nine_ball_break_and_basic_play() {
        let (mut r, mut w) = nine_ball();
        assert!(
            (w.ball(9).unwrap().pos - FOOT_SPOT).len() < 0.001,
            "the 9 sits on the foot spot"
        );
        let ctx = r.context(&w);
        assert_eq!(ctx.legal, vec![1]);
        let mut soft = log(Some(1), &[], false);
        soft.balls_to_cushion = 2;
        assert!(
            r.judge(&ctx, &NONE, &soft, &w).foul.is_some(),
            "soft break is a foul"
        );
        let out = r.judge(&ctx, &NONE, &log(Some(4), &[9], false), &w);
        assert_eq!(out.foul, Some("Wrong ball first"));
        assert_eq!(out.respot, vec![9]);
        assert_eq!(
            r.judge(&ctx, &NONE, &log(Some(1), &[9], false), &w).winner,
            Some(0)
        );
        pot(&mut w, &[1]);
        let out = r.judge(&ctx, &NONE, &log(Some(1), &[1], false), &w);
        assert_eq!((out.foul, out.next_player), (None, 0));
        r.apply(&mut w, &out);
        assert_eq!(r.context(&w).legal, vec![2]);
        assert!(r.push_available);
    }

    #[test]
    fn push_out_suspends_two_fouls_and_lets_the_opponent_choose() {
        let (mut r, mut w) = nine_ball();
        let out = r.judge(&r.context(&w), &NONE, &log(Some(1), &[], false), &w);
        r.apply(&mut w, &out);
        assert!(r.current == 1 && r.push_available);
        let ctx = r.context(&w);
        assert!(ctx.can_push);
        let push = ShotCall {
            push_out: true,
            ..ShotCall::default()
        };
        // No ball hit, no rail, the 9 goes down: still no foul, 9 comes back.
        let mut l = log(None, &[9], false);
        l.cushion_after_contact = false;
        let out = r.judge(&ctx, &push, &l, &w);
        assert_eq!((out.foul, out.winner), (None, None));
        assert_eq!(out.respot, vec![9]);
        let d = out.decision.clone().unwrap();
        assert_eq!(
            (d.by, d.options.clone()),
            (0, vec![Choice::ShootAfterPush, Choice::PassAfterPush])
        );
        r.apply(&mut w, &out);
        assert!(!r.push_available);
        r.choose(&mut w, Choice::PassAfterPush);
        assert_eq!(r.current, 1);
        // A scratch on a push out is still a foul.
        assert_eq!(
            r.judge(&ctx, &push, &log(None, &[], true), &w).foul,
            Some("Scratch")
        );
    }

    #[test]
    fn three_consecutive_fouls_lose_the_rack() {
        let (mut r, mut w) = nine_ball();
        r.break_shot = false;
        r.ball_in_hand = false;
        let foul = log(None, &[], false);
        for expected in [1u8, 2] {
            r.current = 0;
            let out = r.judge(&r.context(&w), &NONE, &foul, &w);
            assert_eq!(out.winner, None);
            r.apply(&mut w, &out);
            assert_eq!(r.fouls[0], expected);
        }
        r.current = 0;
        let out = r.judge(&r.context(&w), &NONE, &foul, &w);
        assert_eq!(out.winner, Some(1));
        // A legal shot in between clears the count.
        r.fouls = [2, 0];
        let ok = r.judge(&r.context(&w), &NONE, &log(Some(1), &[], false), &w);
        r.apply(&mut w, &ok);
        assert_eq!(r.fouls[0], 0);
    }

    #[test]
    fn stalemate_calls_for_a_new_rack() {
        let (mut r, mut w) = eight_ball();
        r.break_shot = false;
        r.ball_in_hand = false;
        r.groups = [Some(Group::Solids), Some(Group::Stripes)];
        for _ in 0..STALEMATE_INNINGS {
            assert_eq!(r.rerack, None);
            let first = r.context(&w).legal[0];
            let out = r.judge(&r.context(&w), &NONE, &log(Some(first), &[], false), &w);
            r.apply(&mut w, &out);
        }
        assert_eq!(r.rerack, Some(0));
    }
}
