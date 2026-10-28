//! Console `record` / `playdemo` / `stop` — deferred to [`Host::frame`].

use std::sync::Mutex;

use deerpunk_engine_common::LightstyleTable;

use crate::cmd::CommandRegistration;
use crate::demo;
use crate::Host;

enum PendingDemoCmd {
    Record(String),
    Play(String),
    Stop,
}

static PENDING: Mutex<Vec<PendingDemoCmd>> = Mutex::new(Vec::new());

pub fn register_commands(cmd: &mut crate::cmd::CmdSystem) {
    cmd.register(CommandRegistration::new("record").execute(|ctx| {
        let Some(name) = ctx.args.first().copied() else {
            println!("Usage: record <name>");
            return;
        };
        PENDING.lock().unwrap().push(PendingDemoCmd::Record(name.to_string()));
    }));
    cmd.register(CommandRegistration::new("playdemo").execute(|ctx| {
        let Some(name) = ctx.args.first().copied() else {
            println!("Usage: playdemo <name>");
            return;
        };
        PENDING.lock().unwrap().push(PendingDemoCmd::Play(name.to_string()));
    }));
    cmd.register(CommandRegistration::new("play").execute(|ctx| {
        let Some(name) = ctx.args.first().copied() else {
            println!("Usage: play <name>  (demo)");
            return;
        };
        PENDING.lock().unwrap().push(PendingDemoCmd::Play(name.to_string()));
    }));
    cmd.register(CommandRegistration::new("stop").execute(|_| {
        PENDING.lock().unwrap().push(PendingDemoCmd::Stop);
    }));
}

impl Host {
    pub(crate) fn drain_pending_demo_cmds(&mut self) {
        let cmds = std::mem::take(&mut *PENDING.lock().unwrap());
        for cmd in cmds {
            match cmd {
                PendingDemoCmd::Record(name) => self.demo_start_record(&name),
                PendingDemoCmd::Play(name) => self.demo_start_play(&name),
                PendingDemoCmd::Stop => self.demo_stop(),
            }
        }
    }

    fn demo_start_record(&mut self, name: &str) {
        let Some(name) = demo::sanitize_name(name) else {
            println!("[demo] bad name (use alphanumeric/_/-)");
            return;
        };
        if self.client_engine.as_mut().unwrap().demo_player.is_some() {
            self.demo_stop();
        }
        if self.client_engine.as_mut().unwrap().demo_writer.is_some() {
            self.demo_stop();
        }
        let path = demo::demo_path(self.fs.gamedir(), &name);
        match demo::DemoWriter::create(path, self.time_secs) {
            Ok(w) => {
                println!("[demo] recording {}", w.path.display());
                self.client_engine.as_mut().unwrap().demo_writer = Some(w);
                if self.listen_server_enabled() {
                    self.connect_to_server_inprocess();
                }
            }
            Err(e) => println!("[demo] record failed: {e}"),
        }
    }

    fn demo_start_play(&mut self, name: &str) {
        let Some(name) = demo::sanitize_name(name) else {
            println!("[demo] bad name (use alphanumeric/_/-)");
            return;
        };
        if self.client_engine.as_mut().unwrap().demo_writer.is_some() {
            self.demo_stop();
        }
        if self.client_engine.as_mut().unwrap().demo_player.is_some() {
            self.demo_stop();
        }
        let path = demo::demo_path(self.fs.gamedir(), &name);
        match demo::DemoPlayer::load(&path) {
            Ok(player) => {
                if self.listen_server_enabled() {
                    self.set_server_enabled(false);
                }
                if self.client_net_active() {
                    self.disconnect_all_net();
                }
                self.flush_client_scene();
                self.client_engine.as_mut().unwrap().lightstyles = LightstyleTable::hl_defaults();
                let n = player.records.len();
                println!("[demo] playing {} ({} records)", path.display(), n);
                self.client_engine.as_mut().unwrap().demo_player = Some(player);
                let _ = self.cvar.set("cl_demoplaying", "1");
                deerpunk_ui::set_client_in_game(true);
            }
            Err(e) => println!("[demo] play failed: {e}"),
        }
    }

    pub(crate) fn demo_stop(&mut self) {
        if let Some(w) = self.client_engine.as_mut().unwrap().demo_writer.take() {
            match w.finish() {
                Ok((path, n)) => {
                    println!("[demo] stopped recording {} ({} pkts)", path.display(), n)
                }
                Err(e) => println!("[demo] stop record error: {e}"),
            }
        }
        if self.client_engine.as_mut().unwrap().demo_player.take().is_some() {
            println!("[demo] stopped playback");
            self.flush_client_scene();
            let _ = self.cvar.set("cl_demoplaying", "0");
            if !self.listen_server_enabled() && !self.client_net_active() {
                deerpunk_ui::set_client_in_game(false);
            }
        }
    }
}
