//! Faith's sounds from the player's own Mirror's Edge install. Which sound plays
//! when is Faith Runner's `faith_anim::sound::Director` (animation cues: footsteps,
//! hand grabs, cloth, breath; landings, rolls, slides, the rush of wind at speed).
//! They play on their own audio output (rodio), on a thread of their own.
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use faith_anim::sound::{CueInfo, SoundCmd};
use me_assets::{Packages, SoundBank};
use rodio::{Decoder, OutputStream, Sink, Source};

use crate::log;

pub enum Msg {
    /// Read these cues; the infos come back on `reply`.
    Load { cooked: PathBuf, wanted: Vec<String>, reply: Sender<HashMap<String, CueInfo>> },
    Cmd(SoundCmd),
    /// Stop every looping sound (leaving Faith mode).
    StopAll,
    /// New volumes from the menu.
    Volumes(Volumes),
}

/// Volume per kind of sound, times the overall volume.
#[derive(Clone, Copy, Debug)]
pub struct Volumes {
    pub all: f32,
    pub footsteps: f32,
    pub hands: f32,
    pub voice: f32,
    pub effects: f32,
}

impl Volumes {
    /// By the Mirror's Edge package the cue comes from.
    fn of(&self, cue: &str) -> f32 {
        self.all
            * if cue.starts_with("a_material_footstep") {
                self.footsteps
            } else if cue.starts_with("a_material_handstep") {
                self.hands
            } else if cue.starts_with("a_character_female") {
                self.voice
            } else {
                self.effects
            }
    }
}

pub fn start(volume: Volumes) -> Sender<Msg> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("faithrunner-audio".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(rx, volume)));
            log!("audio: stopped");
        })
        .expect("audio thread");
    tx
}

fn run(rx: Receiver<Msg>, mut volume: Volumes) {
    let Ok((_stream, handle)) = OutputStream::try_default() else {
        log!("audio: no sound output device");
        return;
    };
    let mut waves: HashMap<String, Vec<Vec<u8>>> = HashMap::new();
    let mut loops: HashMap<u64, (Sink, f32)> = HashMap::new();
    for msg in rx {
        match msg {
            Msg::Load { cooked, wanted, reply } => {
                let mut bank = SoundBank::new(Packages::new(&cooked));
                let mut infos = HashMap::new();
                for path in wanted {
                    let key = path.to_ascii_lowercase();
                    if infos.contains_key(&key) {
                        continue;
                    }
                    let Some(cue) = bank.cue(&path) else { continue };
                    let w: Vec<Vec<u8>> = cue.waves.iter().map(|w| w.ogg.to_vec()).collect();
                    infos.insert(key.clone(), CueInfo { variants: w.len(), volume: cue.volume, pitch: cue.pitch, looping: cue.looping });
                    waves.insert(key, w);
                }
                log!("audio: {} of Faith's sounds loaded", infos.len());
                let _ = reply.send(infos);
            }
            Msg::Cmd(SoundCmd::Play { id, cue, variant, volume: v, speed, looping }) => {
                let Some(ogg) = waves.get(&cue).and_then(|w| w.get(variant)) else { continue };
                let Ok(sink) = Sink::try_new(&handle) else { continue };
                let Ok(dec) = Decoder::new(Cursor::new(ogg.clone())) else { continue };
                let gain = volume.of(&cue);
                sink.set_volume(v * gain);
                sink.set_speed(speed);
                if looping {
                    sink.append(dec.buffered().repeat_infinite());
                    loops.insert(id, (sink, gain));
                } else {
                    sink.append(dec);
                    sink.detach();
                }
            }
            Msg::Cmd(SoundCmd::Stop(id)) => {
                if let Some((s, _)) = loops.remove(&id) {
                    s.stop();
                }
            }
            Msg::Cmd(SoundCmd::Volume(id, v)) => {
                if let Some((s, gain)) = loops.get(&id) {
                    s.set_volume(v * gain);
                }
            }
            Msg::Volumes(v) => volume = v,
            Msg::StopAll => {
                for (_, (s, _)) in loops.drain() {
                    s.stop();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs ME_INSTALL: Faith's cues read and decode.
    #[test]
    fn faith_sounds_decode() {
        let Some(dir) = std::env::var_os("ME_INSTALL") else { return };
        let arms = me_assets::FaithArms::load(std::path::Path::new(&dir), 64).expect("load");
        let wanted = faith_anim::sound::wanted_cues(Some(&arms));
        let mut bank = SoundBank::new(Packages::new(&arms.cooked_pc));
        let (mut cues, mut decoded) = (0, 0);
        for path in &wanted {
            let Some(cue) = bank.cue(path) else { continue };
            cues += 1;
            if let Some(w) = cue.waves.first()
                && let Ok(d) = Decoder::new(Cursor::new(w.ogg.to_vec()))
            {
                assert!(d.sample_rate() > 0);
                decoded += 1;
            }
        }
        println!("{} cues wanted, {cues} read, {decoded} decode", wanted.len());
        assert!(decoded > 20 && decoded == cues);
    }
}
