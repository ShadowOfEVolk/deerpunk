//! Local/listen player connections (not ECS entities).

use deerpunk_engine_common::network::UserCmd;
use deerpunk_engine_common::world::EntityId;

#[derive(Debug, Clone)]
pub struct PlayerConnection {
    pub slot: u32,
    pub name: String,
    pub local: bool,
    /// Filled after gamelib spawns the player entity (optional).
    pub entity: Option<EntityId>,
    /// Latest UserCmd from this connection (listen / net).
    pub usercmd: UserCmd,
}

#[derive(Default)]
pub struct PlayerConnections {
    players: Vec<PlayerConnection>,
}

impl PlayerConnections {
    pub fn clear(&mut self) {
        self.players.clear();
    }

    pub fn list(&self) -> &[PlayerConnection] {
        &self.players
    }

    pub fn get(&self, slot: u32) -> Option<&PlayerConnection> {
        self.players.iter().find(|p| p.slot == slot)
    }

    pub fn get_mut(&mut self, slot: u32) -> Option<&mut PlayerConnection> {
        self.players.iter_mut().find(|p| p.slot == slot)
    }

    pub fn local_slot(&self) -> Option<u32> {
        self.players.iter().find(|p| p.local).map(|p| p.slot)
    }

    /// Insert local listen client on slot 0 (idempotent if already present).
    pub fn connect_local(&mut self, name: impl Into<String>) -> Option<&PlayerConnection> {
        if self.local_slot().is_some() {
            return self.players.iter().find(|p| p.local);
        }
        let slot = 0;
        self.players.push(PlayerConnection {
            slot,
            name: name.into(),
            local: true,
            entity: None,
            usercmd: UserCmd::default(),
        });
        log::info!("[players] local connect slot={slot}");
        self.players.iter().find(|p| p.slot == slot)
    }

    pub fn disconnect_local(&mut self) -> Option<PlayerConnection> {
        let idx = self.players.iter().position(|p| p.local)?;
        let p = self.players.remove(idx);
        log::info!("[players] local disconnect slot={}", p.slot);
        Some(p)
    }

    /// Remote (UDP) client on a free slot.
    pub fn connect_remote(&mut self, slot: u32, name: impl Into<String>) -> Option<&PlayerConnection> {
        if self.players.iter().any(|p| p.slot == slot) {
            return self.players.iter().find(|p| p.slot == slot);
        }
        self.players.push(PlayerConnection {
            slot,
            name: name.into(),
            local: false,
            entity: None,
            usercmd: UserCmd::default(),
        });
        log::info!("[players] remote connect slot={slot}");
        self.players.iter().find(|p| p.slot == slot)
    }

    pub fn disconnect_slot(&mut self, slot: u32) -> Option<PlayerConnection> {
        let idx = self.players.iter().position(|p| p.slot == slot)?;
        let p = self.players.remove(idx);
        log::info!("[players] disconnect slot={}", p.slot);
        Some(p)
    }

    pub fn slots(&self) -> Vec<u32> {
        self.players.iter().map(|p| p.slot).collect()
    }

    pub fn set_entity(&mut self, slot: u32, entity: EntityId) {
        if let Some(p) = self.get_mut(slot) {
            p.entity = Some(entity);
            log::info!("[players] slot={slot} entity={entity:#x}");
        }
    }

    pub fn set_usercmd(&mut self, slot: u32, cmd: UserCmd) {
        if let Some(p) = self.get_mut(slot) {
            p.usercmd = cmd;
        }
    }

    pub fn usercmd(&self, slot: u32) -> Option<UserCmd> {
        self.get(slot).map(|p| p.usercmd)
    }
}
