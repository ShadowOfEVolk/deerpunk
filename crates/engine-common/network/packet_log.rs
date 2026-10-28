//! Human-readable SVC/CLC lines for [`net_showpackets`] (Xash-style).

use super::protocol::{CLC, SVC};
use super::stream_rep::StreamInRpc;
use super::user_message::UserMessageRpc;

/// Log outbound packets when non-zero and not `2` (Xash: hide `-->`).
#[inline]
pub fn should_log_out(level: f32) -> bool {
    level != 0.0 && level != 2.0
}

/// Log inbound packets when non-zero and not `3` (Xash: hide `<--`).
#[inline]
pub fn should_log_in(level: f32) -> bool {
    level != 0.0 && level != 3.0
}

/// At `4`, skip high-frequency cmds (still log connect/init/stream.in).
pub fn should_log_svc(level: f32, cmd: u8, inbound: bool) -> bool {
    if inbound {
        if !should_log_in(level) {
            return false;
        }
    } else if !should_log_out(level) {
        return false;
    }
    if level < 4.0 {
        return true;
    }
    !matches!(
        cmd,
        x if x == SVC::NetDelta as u8
            || x == SVC::StreamUpdate as u8
            || x == SVC::SoundPlay as u8
    )
}

pub fn should_log_clc(level: f32, cmd: u8, inbound: bool) -> bool {
    if inbound {
        if !should_log_in(level) {
            return false;
        }
    } else if !should_log_out(level) {
        return false;
    }
    if level < 4.0 {
        return true;
    }
    cmd != CLC::UserCmd as u8
}

pub fn svc_label(cmd: u8) -> &'static str {
    match SVC::from_u8(cmd) {
        Some(SVC::Nop) => "SVC::Nop",
        Some(SVC::Bad) => "SVC::Bad",
        Some(SVC::SoundPlay) => "SVC::SoundPlay",
        Some(SVC::SpawnPrefab) => "SVC::SpawnPrefab",
        Some(SVC::NetDelta) => "SVC::NetDelta",
        Some(SVC::NetDestroy) => "SVC::NetDestroy",
        Some(SVC::UserMessage) => "SVC::UserMessage",
        Some(SVC::PrefabDef) => "SVC::PrefabDef",
        Some(SVC::InitDone) => "SVC::InitDone",
        Some(SVC::SoundStop) => "SVC::SoundStop",
        Some(SVC::StreamIn) => "SVC::StreamIn",
        Some(SVC::StreamOut) => "SVC::StreamOut",
        Some(SVC::StreamUpdate) => "SVC::StreamUpdate",
        Some(SVC::UserCmdAck) => "SVC::UserCmdAck",
        None => "SVC::?",
    }
}

pub fn clc_label(cmd: u8) -> &'static str {
    match CLC::from_u8(cmd) {
        Some(CLC::Nop) => "CLC::Nop",
        Some(CLC::Bad) => "CLC::Bad",
        Some(CLC::UserCmd) => "CLC::UserCmd",
        None => "CLC::?",
    }
}

/// Extra fields after the command name (empty if unknown cmd).
pub fn describe_svc_payload(packet: &[u8]) -> String {
    let Some(&cmd) = packet.first() else {
        return "empty".into();
    };
    let payload = packet.get(1..).unwrap_or(&[]);
    match SVC::from_u8(cmd) {
        Some(SVC::StreamIn) => {
            if let Some(r) = StreamInRpc::decode(payload) {
                format!(
                    "net_id={} gen={} prefab='{}' components={} hot={}",
                    r.net_id,
                    r.generation,
                    r.prefab,
                    r.components.len(),
                    r.hot.len()
                )
            } else {
                "decode_err".into()
            }
        }
        Some(SVC::UserMessage) => {
            if let Some(r) = UserMessageRpc::decode(payload) {
                format!("msg_id={} data={}", r.msg_id, r.data.len())
            } else {
                "decode_err".into()
            }
        }
        Some(SVC::InitDone) => String::new(),
        Some(SVC::NetDelta) | Some(SVC::StreamUpdate) | Some(SVC::NetDestroy) => {
            format!("payload={}", payload.len())
        }
        _ => {
            if payload.is_empty() {
                String::new()
            } else {
                format!("payload={}", payload.len())
            }
        }
    }
}

pub fn format_svc_line(packet: &[u8]) -> String {
    let cmd = packet.first().copied().unwrap_or(255);
    let extra = describe_svc_payload(packet);
    if extra.is_empty() {
        svc_label(cmd).to_string()
    } else {
        format!("{} {}", svc_label(cmd), extra)
    }
}

pub fn format_clc_line(packet: &[u8]) -> String {
    let cmd = packet.first().copied().unwrap_or(255);
    let payload_len = packet.len().saturating_sub(1);
    match CLC::from_u8(cmd) {
        Some(CLC::UserCmd) => format!("{} payload={}", clc_label(cmd), payload_len),
        _ => {
            if payload_len == 0 {
                clc_label(cmd).to_string()
            } else {
                format!("{} payload={}", clc_label(cmd), payload_len)
            }
        }
    }
}
