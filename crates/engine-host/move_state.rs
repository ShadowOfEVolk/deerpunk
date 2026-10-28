//! Client move intent from `+forward` / `-forward` style commands.

use std::sync::Mutex;

use deerpunk_engine_common::network::{
    AXIS_COUNT, AXIS_FORWARD, AXIS_SIDE, AXIS_UP, BUTTON_ATTACK, BUTTON_DUCK, BUTTON_DROP,
    BUTTON_JUMP, BUTTON_ATTACK2, BUTTON_ATTACK3, BUTTON_SPEED, BUTTON_USE,
};

use crate::cmd::{CmdSystem, CommandRegistration};

#[derive(Clone, Copy, Debug, Default)]
pub struct MoveState {
    pub forward: i8,
    pub back: i8,
    pub moveleft: i8,
    pub moveright: i8,
    pub moveup: i8,
    pub movedown: i8,
    pub buttons: u64,
    /// One-shot bits merged into the next [`snapshot`], then cleared (for `bind q drop`).
    impulse: u64,
}

impl MoveState {
    pub fn axes(&self) -> [f32; AXIS_COUNT] {
        let mut a = [0.0f32; AXIS_COUNT];
        a[AXIS_FORWARD] = (self.forward as f32) - (self.back as f32);
        a[AXIS_SIDE] = (self.moveright as f32) - (self.moveleft as f32);
        a[AXIS_UP] = (self.moveup as f32) - (self.movedown as f32);
        a
    }

    pub fn clear_keys(&mut self) {
        self.forward = 0;
        self.back = 0;
        self.moveleft = 0;
        self.moveright = 0;
        self.moveup = 0;
        self.movedown = 0;
        self.buttons = 0;
        self.impulse = 0;
    }
}

/// Axes + button mask for one outgoing usercmd (clears [`MoveState::impulse`]).
pub fn snapshot() -> ([f32; AXIS_COUNT], u64) {
    let g = global();
    let buttons = g.buttons | g.impulse;
    with_mut(|m| m.impulse = 0);
    (g.axes(), buttons)
}

static MOVE: Mutex<MoveState> = Mutex::new(MoveState {
    forward: 0,
    back: 0,
    moveleft: 0,
    moveright: 0,
    moveup: 0,
    movedown: 0,
    buttons: 0,
    impulse: 0,
});

pub fn global() -> MoveState {
    *MOVE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_global(state: MoveState) {
    *MOVE.lock().unwrap_or_else(|e| e.into_inner()) = state;
}

fn with_mut(f: impl FnOnce(&mut MoveState)) {
    let mut g = MOVE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g);
}

fn set_axis_key(key: &str, on: bool) {
    with_mut(|m| {
        let v = if on { 1 } else { 0 };
        match key {
            "forward" => m.forward = v,
            "back" => m.back = v,
            "moveleft" => m.moveleft = v,
            "moveright" => m.moveright = v,
            "moveup" => m.moveup = v,
            "movedown" => m.movedown = v,
            _ => {}
        }
    });
}

fn set_button(bit: u64, on: bool) {
    with_mut(|m| {
        if on {
            m.buttons |= bit;
        } else {
            m.buttons &= !bit;
        }
    });
}

fn pulse_button(bit: u64) {
    with_mut(|m| m.impulse |= bit);
}

/// Register `+forward`/`-forward`/… and button cmds on the host console.
pub fn register_commands(cmd: &mut CmdSystem) {
    macro_rules! axis_pair {
        ($name:literal) => {{
            let plus = concat!("+", $name);
            let minus = concat!("-", $name);
            cmd.register(
                CommandRegistration::new(plus).execute(|_| set_axis_key($name, true)),
            );
            cmd.register(
                CommandRegistration::new(minus).execute(|_| set_axis_key($name, false)),
            );
        }};
    }
    axis_pair!("forward");
    axis_pair!("back");
    axis_pair!("moveleft");
    axis_pair!("moveright");
    axis_pair!("moveup");
    axis_pair!("movedown");

    macro_rules! btn_pair {
        ($name:literal, $bit:expr) => {{
            let plus = concat!("+", $name);
            let minus = concat!("-", $name);
            cmd.register(
                CommandRegistration::new(plus).execute(|_| set_button($bit, true)),
            );
            cmd.register(
                CommandRegistration::new(minus).execute(|_| set_button($bit, false)),
            );
        }};
    }
    btn_pair!("attack", BUTTON_ATTACK);
    btn_pair!("attack2", BUTTON_ATTACK2);
    btn_pair!("attack3", BUTTON_ATTACK3);
    btn_pair!("jump", BUTTON_JUMP);
    btn_pair!("duck", BUTTON_DUCK);
    btn_pair!("use", BUTTON_USE);
    btn_pair!("speed", BUTTON_SPEED);
    btn_pair!("drop", BUTTON_DROP);
    cmd.register(
        CommandRegistration::new("drop").execute(|_| pulse_button(BUTTON_DROP)),
    );
}

/// Clear held move keys (console open / disconnect).
pub fn clear() {
    with_mut(|m| m.clear_keys());
}
