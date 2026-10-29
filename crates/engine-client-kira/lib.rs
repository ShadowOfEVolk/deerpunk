//! Kira audio backend — datapack `sound_events` / `sound_banks` + clips via [`ResourceManager`].

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use deerpunk_engine_client::audio::{AudioBackend, AudioInitContext, AudioListener, FmodEventInfo};
use deerpunk_engine_common::resource::{path_to_file_uri, AudioClip, ResourceManager};
use deerpunk_engine_common::sound_event::{
    normalize_event_id, ParamLerp, SoundEventBank, SoundEventDef, SoundEventRegistry,
};
use kira::manager::{
    backend::DefaultBackend,
    AudioManager, AudioManagerSettings,
};
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::FromFileError;
use kira::tween::Tween;
use rand::Rng;
use walkdir::WalkDir;

pub struct KiraBackend {
    manager: Option<AudioManager<DefaultBackend>>,
    events: SoundEventRegistry,
    gamedir: PathBuf,
    clip_cache: HashMap<String, StaticSoundData>,
    attached: HashMap<u64, kira::sound::static_sound::StaticSoundHandle>,
    /// Looping 2D instances keyed by host `sound_id` (music — streamed from disk).
    loop_streams: HashMap<u32, StreamingSoundHandle<FromFileError>>,
    last_volume: f32,
    last_listener: AudioListener,
}

impl Default for KiraBackend {
    fn default() -> Self {
        Self {
            manager: None,
            events: SoundEventRegistry::default(),
            gamedir: PathBuf::new(),
            clip_cache: HashMap::new(),
            attached: HashMap::new(),
            loop_streams: HashMap::new(),
            last_volume: 1.0,
            last_listener: AudioListener::default(),
        }
    }
}

impl KiraBackend {
    fn spatial_gain_pan(
        listener_pos: [f32; 3],
        listener_fwd: [f32; 3],
        listener_up: [f32; 3],
        origin: [f32; 3],
    ) -> (f32, f64) {
        let dx = origin[0] - listener_pos[0];
        let dy = origin[1] - listener_pos[1];
        let dz = origin[2] - listener_pos[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(0.001);
        // Inverse-distance with reference distance ~6 m (game units).
        let atten = (6.0 / (6.0 + dist)).clamp(0.05, 1.0);

        let fwd_len = (listener_fwd[0] * listener_fwd[0]
            + listener_fwd[1] * listener_fwd[1]
            + listener_fwd[2] * listener_fwd[2])
            .sqrt()
            .max(0.001);
        let fx = listener_fwd[0] / fwd_len;
        let fy = listener_fwd[1] / fwd_len;
        let fz = listener_fwd[2] / fwd_len;

        let up_len = (listener_up[0] * listener_up[0]
            + listener_up[1] * listener_up[1]
            + listener_up[2] * listener_up[2])
            .sqrt()
            .max(0.001);
        let ux = listener_up[0] / up_len;
        let uy = listener_up[1] / up_len;
        let uz = listener_up[2] / up_len;

        // right = forward × up (camera basis)
        let rx = fy * uz - fz * uy;
        let ry = fz * ux - fx * uz;
        let rz = fx * uy - fy * ux;
        let r_len = (rx * rx + ry * ry + rz * rz).sqrt().max(0.001);

        let side = (dx * rx + dy * ry + dz * rz) / (dist * r_len);
        let elev = (dy / dist).clamp(-0.35, 0.35);
        // Kira panning is 0 = left, 0.5 = center, 1 = right (not -1..1).
        let stereo = (side + elev * 0.12).clamp(-1.0, 1.0);
        let kira_pan = ((stereo as f64 + 1.0) * 0.5).clamp(0.0, 1.0);
        (atten, kira_pan)
    }

    fn clips_for_def(def: &SoundEventDef) -> Vec<&str> {
        match def {
            SoundEventDef::Single { clip, .. } => vec![clip.as_str()],
            SoundEventDef::Random { clips, .. } => clips.iter().map(String::as_str).collect(),
        }
    }

    /// Decode short SFX clips once at startup so gameplay plays don't hitch the mixer.
    fn warm_sfx_clip_cache(&mut self) {
        let mut to_load: Vec<(String, PathBuf)> = Vec::new();
        for def in self.events.events.values() {
            for clip_rel in Self::clips_for_def(def) {
                if clip_rel.contains("/music/") || clip_rel.contains("\\music\\") {
                    continue;
                }
                let uri = self.resolve_clip_uri(clip_rel);
                if self.clip_cache.contains_key(&uri) {
                    continue;
                }
                to_load.push((uri, self.resolve_clip_path(clip_rel)));
            }
        }
        for (uri, path) in to_load {
            match StaticSoundData::from_file(&path) {
                Ok(d) => {
                    self.clip_cache.insert(uri, d);
                }
                Err(e) => log::warn!("[kira] warm {}: {e}", path.display()),
            }
        }
    }

    fn resolve_clip_path(&self, clip: &str) -> PathBuf {
        if clip.starts_with("file://") {
            let rest = clip.strip_prefix("file://").unwrap_or(clip).trim_start_matches('/');
            return PathBuf::from(rest);
        }
        let rel = clip.trim_start_matches('/');
        self.gamedir.join(rel)
    }

    fn resolve_clip_uri(&self, clip: &str) -> String {
        if clip.starts_with("file://") {
            return clip.to_string();
        }
        let rel = clip.trim_start_matches('/');
        let path = self.gamedir.join(rel);
        path_to_file_uri(&path)
    }

    fn load_sound_data(
        &mut self,
        assets: &ResourceManager,
        uri: &str,
    ) -> Option<StaticSoundData> {
        if let Some(cached) = self.clip_cache.get(uri) {
            return Some(cached.clone());
        }
        let handle = assets.load::<AudioClip>(uri);
        let clip = handle.try_get()?;
        let cursor = Cursor::new(clip.bytes);
        let data = StaticSoundData::from_cursor(cursor);
        match data {
            Ok(d) => {
                self.clip_cache.insert(uri.to_string(), d.clone());
                Some(d)
            }
            Err(e) => {
                log::warn!("[kira] decode {uri}: {e}");
                None
            }
        }
    }

    fn apply_params(
        def: &SoundEventDef,
        params: &[(&str, f32)],
        base_vol: f32,
        base_pitch: f32,
    ) -> (f32, f32) {
        let mut vol = base_vol;
        let mut pitch = base_pitch;
        let map = def.params_map();
        for (name, value) in params {
            if let Some(binding) = map.get(*name) {
                if let Some(l) = binding.volume {
                    vol *= lerp_param(l, *value);
                }
                if let Some(l) = binding.pitch {
                    pitch *= lerp_param(l, *value);
                }
            }
        }
        (vol, pitch)
    }

    fn play_streaming_loop(
        &mut self,
        def: &SoundEventDef,
        volume: f32,
        params: &[(&str, f32)],
        instance_id: u32,
    ) -> bool {
        let mut rng = rand::thread_rng();
        let clip_rel = match def.pick_clip(&mut rng) {
            Some(c) => c,
            None => return false,
        };
        let path = self.resolve_clip_path(clip_rel);
        let manager = match self.manager.as_mut() {
            Some(m) => m,
            None => return false,
        };

        let pr = def.pitch_range();
        let pitch = if pr.min >= pr.max {
            pr.min
        } else {
            rng.gen_range(pr.min..=pr.max)
        };
        let (vol_mul, pitch_mul) = Self::apply_params(def, params, def.base_volume() * volume, pitch);

        let mut data = match StreamingSoundData::from_file(&path) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("[kira] stream open {}: {e}", path.display());
                return false;
            }
        };
        data = data
            .loop_region(..)
            .volume(vol_mul.clamp(0.0, 2.0) as f64)
            .playback_rate(pitch_mul.clamp(0.1, 4.0) as f64);

        if let Some(mut old) = self.loop_streams.remove(&instance_id) {
            let _ = old.stop(Tween::default());
        }

        match manager.play(data) {
            Ok(handle) => {
                self.loop_streams.insert(instance_id, handle);
                true
            }
            Err(e) => {
                log::warn!("[kira] stream play {}: {e}", path.display());
                false
            }
        }
    }

    fn play_clip(
        &mut self,
        assets: &ResourceManager,
        def: &SoundEventDef,
        origin: Option<[f32; 3]>,
        volume: f32,
        params: &[(&str, f32)],
        listener: &AudioListener,
        r#loop: bool,
    ) -> bool {
        let mut rng = rand::thread_rng();
        let clip_rel = match def.pick_clip(&mut rng) {
            Some(c) => c,
            None => return false,
        };
        let uri = self.resolve_clip_uri(clip_rel);
        let data = match self.load_sound_data(assets, &uri) {
            Some(d) => d,
            None => return false,
        };
        let manager = match self.manager.as_mut() {
            Some(m) => m,
            None => return false,
        };

        let pr = def.pitch_range();
        let pitch = if pr.min >= pr.max {
            pr.min
        } else {
            rng.gen_range(pr.min..=pr.max)
        };
        let (vol_mul, pitch_mul) = Self::apply_params(def, params, def.base_volume() * volume, pitch);

        let mut settings = StaticSoundSettings::new()
            .volume(vol_mul.clamp(0.0, 2.0) as f64)
            .playback_rate(pitch_mul.clamp(0.1, 4.0) as f64);

        if let Some(origin) = origin {
            let (atten, pan) =
                Self::spatial_gain_pan(listener.position, listener.forward, listener.up, origin);
            settings = settings
                .volume((vol_mul * atten).clamp(0.0, 2.0) as f64)
                .panning(pan);
        }

        if r#loop {
            settings = settings.loop_region(..);
        }

        match manager.play(data.with_settings(settings)) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("[kira] play {uri}: {e}");
                false
            }
        }
    }

    fn play_event_inner(
        &mut self,
        assets: &ResourceManager,
        event: &str,
        origin: Option<[f32; 3]>,
        volume: f32,
        params: &[(&str, f32)],
        force_loop: bool,
        instance_id: u32,
    ) {
        let id = normalize_event_id(event);
        let def = match self.events.events.get(id) {
            Some(d) => d.clone(),
            None => {
                log::warn!("[kira] unknown event '{id}' (from '{event}')");
                return;
            }
        };
        let r#loop = force_loop || def.is_loop();
        if r#loop && instance_id != 0 && origin.is_none() {
            let _ = self.play_streaming_loop(&def, volume, params, instance_id);
            return;
        }
        let listener = self.last_listener;
        self.play_clip(assets, &def, origin, volume, params, &listener, r#loop);
    }
}

fn lerp_param(l: ParamLerp, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    l.min + (l.max - l.min) * t
}

fn discover_datapack_events(gamedir: &Path) -> SoundEventRegistry {
    let mut out = SoundEventRegistry::default();
    let datapacks = gamedir.join("datapacks");
    if !datapacks.is_dir() {
        return out;
    }
    for entry in WalkDir::new(&datapacks).min_depth(2).max_depth(6) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str());
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !fname.ends_with(".json") {
            continue;
        }
        let data = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[kira] read {}: {e}", path.display());
                continue;
            }
        };
        match parent {
            Some("sound_events") => {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(fname);
                match serde_json::from_str::<SoundEventDef>(&data) {
                    Ok(def) => {
                        log::info!("[kira] event '{}' <- {}", stem, path.display());
                        out.insert(stem, def);
                    }
                    Err(e) => log::warn!("[kira] bad sound_event {}: {e}", path.display()),
                }
            }
            Some("sound_banks") => match serde_json::from_str::<SoundEventBank>(&data) {
                Ok(bank) => {
                    log::info!(
                        "[kira] bank {} ({} events)",
                        path.display(),
                        bank.events.len()
                    );
                    out.merge_bank(bank);
                }
                Err(e) => log::warn!("[kira] bad sound_bank {}: {e}", path.display()),
            },
            _ => {}
        }
    }
    out
}

impl AudioBackend for KiraBackend {
    fn init(&mut self, ctx: &AudioInitContext) -> bool {
        self.gamedir = ctx.gamedir.to_path_buf();
        let manager = match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("[kira] AudioManager::new failed: {e}");
                return false;
            }
        };
        self.manager = Some(manager);
        self.events = discover_datapack_events(ctx.gamedir);
        self.warm_sfx_clip_cache();
        log::info!(
            "[kira] ready — {} event(s), {} cached clip(s), gamedir={}",
            self.events.events.len(),
            self.clip_cache.len(),
            ctx.gamedir.display()
        );
        true
    }

    fn merge_sound_events(&mut self, registry: &SoundEventRegistry) {
        for (k, v) in &registry.events {
            self.events.insert(k.clone(), v.clone());
        }
        self.warm_sfx_clip_cache();
    }

    fn update(&mut self, listener: &AudioListener, attached: &[(u64, [f32; 3])], volume: f32) {
        self.last_listener = *listener;
        if let Some(m) = self.manager.as_mut() {
            if (volume - self.last_volume).abs() > 0.001 {
                let _ = m.main_track().set_volume(
                    volume.clamp(0.0, 1.0) as f64,
                    Tween::default(),
                );
                self.last_volume = volume;
            }
        }
        for (key, origin) in attached {
            if let Some(handle) = self.attached.get_mut(key) {
                let (atten, pan) =
                    Self::spatial_gain_pan(
                        listener.position,
                        listener.forward,
                        listener.up,
                        *origin,
                    );
                let _ = handle.set_volume(atten.clamp(0.0, 2.0) as f64, Tween::default());
                let _ = handle.set_panning(pan, Tween::default());
            }
        }
    }

    fn play_event_2d(
        &mut self,
        assets: &ResourceManager,
        event: &str,
        volume: f32,
        params: &[(&str, f32)],
        instance_id: u32,
    ) {
        self.play_event_inner(assets, event, None, volume, params, false, instance_id);
    }

    fn stop_instance(&mut self, instance_id: u32) {
        if instance_id == 0 {
            return;
        }
        if let Some(mut h) = self.loop_streams.remove(&instance_id) {
            let _ = h.stop(Tween::default());
        }
    }

    fn play_event_3d(
        &mut self,
        assets: &ResourceManager,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: &[(&str, f32)],
    ) {
        self.play_event_inner(assets, event, Some(origin), volume, params, false, 0);
    }

    fn start_attached(
        &mut self,
        assets: &ResourceManager,
        key: u64,
        event: &str,
        origin: [f32; 3],
        volume: f32,
    ) {
        self.stop_attached(key);
        let id = normalize_event_id(event);
        let def = match self.events.events.get(id).cloned() {
            Some(d) => d,
            None => {
                log::warn!("[kira] unknown attached event '{id}'");
                return;
            }
        };
        let mut rng = rand::thread_rng();
        let clip_rel = match def.pick_clip(&mut rng) {
            Some(c) => c,
            None => return,
        };
        let uri = self.resolve_clip_uri(clip_rel);
        let data = match self.load_sound_data(assets, &uri) {
            Some(d) => d,
            None => return,
        };
        let manager = match self.manager.as_mut() {
            Some(m) => m,
            None => return,
        };
        let pr = def.pitch_range();
        let pitch = rng.gen_range(pr.min..=pr.max);
        let (atten, pan) =
            Self::spatial_gain_pan(
                self.last_listener.position,
                self.last_listener.forward,
                self.last_listener.up,
                origin,
            );
        let settings = StaticSoundSettings::new()
            .volume((def.base_volume() * volume * atten).clamp(0.0, 2.0) as f64)
            .playback_rate(pitch.clamp(0.1, 4.0) as f64)
            .panning(pan)
            .loop_region(..);
        match manager.play(data.with_settings(settings)) {
            Ok(h) => {
                self.attached.insert(key, h);
            }
            Err(e) => log::warn!("[kira] attached {id}: {e}"),
        }
    }

    fn stop_attached(&mut self, key: u64) {
        if let Some(mut h) = self.attached.remove(&key) {
            let _ = h.stop(Tween::default());
        }
    }

    fn shutdown(&mut self) {
        self.attached.clear();
        self.loop_streams.clear();
        self.clip_cache.clear();
        self.manager = None;
    }

    fn list_events(&self) -> Vec<FmodEventInfo> {
        self.events
            .events
            .keys()
            .map(|id| FmodEventInfo {
                path: id.clone(),
                bank: "kira".into(),
                is_3d: true,
                oneshot: true,
                params: Vec::new(),
            })
            .collect()
    }
}
