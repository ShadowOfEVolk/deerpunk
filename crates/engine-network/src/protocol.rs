//! Wire command discriminants (first byte of SVC / CLC packets).

/// Server → client.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SVC {
    /// No-op / padding.
    Nop = 0,
    /// Invalid / unknown (never emitted).
    Bad = 1,
    /// Start / one-shot sound ([`super::sound_event::SoundPlayRpc`], server `sound_id`).
    SoundPlay = 2,
    /// Spawn prefab on client (`SpawnPrefabRpc`).
    /// `net_id == 0` → static/scene (e.g. worldspawn); else NetworkEntity.
    SpawnPrefab = 3,
    /// Hot-field delta for a NetworkEntity (`NetDeltaRpc`).
    NetDelta = 4,
    /// Destroy NetworkEntity on client (`NetDestroyRpc`).
    NetDestroy = 5,
    /// User message (`UserMessageRpc`): `u8 msg_id` + payload.
    UserMessage = 6,
    /// Authoritative prefab JSON (`PrefabDefRpc`) — sent on connect before spawns.
    PrefabDef = 7,
    /// End of join **init** phase; client may enter **play** (UserCmd, full sim UI).
    InitDone = 8,
    /// Stop a playing/looping sound by server [`super::sound_event::SoundStopRpc::sound_id`].
    SoundStop = 9,
    /// Stream entity into client interest (`StreamInRpc`).
    StreamIn = 10,
    /// Stream entity out of client interest (`StreamOutRpc`).
    StreamOut = 11,
    /// Batched hot-field updates (`StreamUpdateBatchRpc`).
    StreamUpdate = 12,
    /// Last processed client `UserCmd::sequence` (`UserCmdAckRpc`).
    UserCmdAck = 13,
}

impl SVC {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Nop),
            1 => Some(Self::Bad),
            2 => Some(Self::SoundPlay),
            3 => Some(Self::SpawnPrefab),
            4 => Some(Self::NetDelta),
            5 => Some(Self::NetDestroy),
            6 => Some(Self::UserMessage),
            7 => Some(Self::PrefabDef),
            8 => Some(Self::InitDone),
            9 => Some(Self::SoundStop),
            10 => Some(Self::StreamIn),
            11 => Some(Self::StreamOut),
            12 => Some(Self::StreamUpdate),
            13 => Some(Self::UserCmdAck),
            _ => None,
        }
    }
}

/// Client → server.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CLC {
    /// No-op / padding.
    Nop = 0,
    /// Invalid / unknown (never emitted).
    Bad = 1,
    /// UserCmd blob (see [`super::usercmd::UserCmd`]).
    /// Format: `[u8; 64]` payload.
    UserCmd = 2,
}

impl CLC {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Nop),
            1 => Some(Self::Bad),
            2 => Some(Self::UserCmd),
            _ => None,
        }
    }
}
