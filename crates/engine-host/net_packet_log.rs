//! [`net_showpackets`] — log SVC/CLC like Xash `net_showpackets`.

use deerpunk_engine_common::cvar_def::ARCHIVE;

deerpunk_engine_common::cvar_def!(
    "net_showpackets",
    "0",
    "Log SVC/CLC packets (Xash-style): 0=off, 1=all, 2=hide outbound (-->), 3=hide inbound (<--), 4=+hide UserCmd/NetDelta/StreamUpdate/SoundPlay",
    ARCHIVE
);

use deerpunk_engine_common::network::{
    format_clc_line, format_svc_line, should_log_clc, should_log_svc,
};

use crate::cvar::CvarSystem;

#[inline]
fn level(cvar: &CvarSystem) -> f32 {
    cvar.get_f32_or("net_showpackets", 0.0)
}

pub fn log_s2c_out(cvar: &CvarSystem, via: &str, packet: &[u8]) {
    let cmd = packet.first().copied().unwrap_or(0);
    let lvl = level(cvar);
    if !should_log_svc(lvl, cmd, false) {
        return;
    }
    log::info!(
        "[net] {} --> sz={} {}",
        via,
        packet.len(),
        format_svc_line(packet)
    );
}

pub fn log_s2c_in(cvar: &CvarSystem, via: &str, packet: &[u8]) {
    let cmd = packet.first().copied().unwrap_or(0);
    let lvl = level(cvar);
    if !should_log_svc(lvl, cmd, true) {
        return;
    }
    log::info!(
        "[net] {} <-- sz={} {}",
        via,
        packet.len(),
        format_svc_line(packet)
    );
}

pub fn log_clc_out(cvar: &CvarSystem, via: &str, packet: &[u8]) {
    let cmd = packet.first().copied().unwrap_or(0);
    let lvl = level(cvar);
    if !should_log_clc(lvl, cmd, false) {
        return;
    }
    log::info!(
        "[net] {} --> sz={} {}",
        via,
        packet.len(),
        format_clc_line(packet)
    );
}

pub fn log_clc_in(cvar: &CvarSystem, slot: u32, packet: &[u8]) {
    let cmd = packet.first().copied().unwrap_or(0);
    let lvl = level(cvar);
    if !should_log_clc(lvl, cmd, true) {
        return;
    }
    log::info!(
        "[net] slot={} <-- sz={} {}",
        slot,
        packet.len(),
        format_clc_line(packet)
    );
}
