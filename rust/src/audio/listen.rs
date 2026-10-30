//! Who records from the cable of microphone processing. With micFx.sleep the processing sleeps while no
//! app does (router::sync): the recording end of the cable is watched — a session created, started,
//! stopped or gone makes the engine look again, and a change wakes the router at once.

use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use windows::{
    Win32::{
        Media::Audio::{
            AudioSessionDisconnectReason, AudioSessionState, AudioSessionStateActive, IAudioSessionControl, IAudioSessionControl2, IAudioSessionEvents, IAudioSessionEvents_Impl,
            IAudioSessionManager2, IAudioSessionNotification, IAudioSessionNotification_Impl,
        },
        System::Com::{CLSCTX_ALL, CoTaskMemFree},
    },
    core::{BOOL, GUID, Interface, PCWSTR, Ref, implement},
};

use super::{Engine, SELF_PID, com, run};
use crate::{appinfo, logf, util};

/// The cable to watch (its playback end, micFx.out); "" — none.
static WANT: Mutex<String> = Mutex::new(String::new());
/// 0 — not known, 1 — no app records from the cable, 2 — some app does (or its recording end is unknown)
static STATE: AtomicU8 = AtomicU8::new(0);
/// a look is queued on the engine thread (the callbacks come in bursts)
static QUEUED: AtomicBool = AtomicBool::new(false);

/// (router::sync) Watches the recording end of this cable ("" — stops watching); changes only.
pub fn watch_cable(out: &str) {
    {
        let mut w = WANT.lock().unwrap_or_else(|p| p.into_inner());
        if *w == out {
            return;
        }
        *w = out.to_string();
    }
    STATE.store(0, Ordering::Relaxed);
    look();
}

/// No app records from the watched cable — known for sure (not known yet: false).
pub fn cable_idle() -> bool {
    STATE.load(Ordering::Relaxed) == 1
}

fn look() {
    if !QUEUED.swap(true, Ordering::AcqRel) {
        run(|e| {
            QUEUED.store(false, Ordering::Release);
            e.listen_look(false);
        });
    }
}

#[derive(Default)]
pub(super) struct Listen {
    /// the cable's playback end being watched, and its recording end
    out: String,
    cap: String,
    mgr: Option<IAudioSessionManager2>,
    notify: Option<IAudioSessionNotification>,
    events: Option<IAudioSessionEvents>,
    /// sessions with the events registered, by instance id
    sessions: HashMap<String, IAudioSessionControl>,
}

impl Listen {
    pub(super) fn stop(&mut self) {
        unsafe {
            if let Some(ev) = &self.events {
                for s in self.sessions.values() {
                    let _ = s.UnregisterAudioSessionNotification(ev);
                }
            }
            if let (Some(m), Some(n)) = (&self.mgr, &self.notify) {
                let _ = m.UnregisterSessionNotification(n);
            }
        }
        self.sessions.clear();
        self.mgr = None;
        self.cap.clear();
        self.out.clear();
    }
}

impl Engine {
    /// Looks at who records from the cable; `resolve` — find its recording end again (devices changed).
    pub(super) fn listen_look(&mut self, resolve: bool) {
        let want = WANT.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if want.is_empty() {
            if !self.listen.out.is_empty() {
                self.listen.stop();
            }
            return;
        }
        if want != self.listen.out || resolve {
            let Some(en) = self.en.get().cloned() else { return };
            let (render, capture) = com::list_audio_devices(&en);
            // "CABLE Input (…)" plays into "CABLE Output (…)" — the one apps record from
            let cap = render
                .iter()
                .find(|r| r.id == want && !r.pair.is_empty())
                .and_then(|r| capture.iter().find(|c| c.is_virtual && c.name == r.pair))
                .map(|c| c.id.clone())
                .unwrap_or_default();
            let l = &mut self.listen;
            if cap != l.cap || want != l.out {
                l.stop();
                l.out = want.clone();
                l.cap = cap.clone();
                if let Some(mgr) = com::get_device(&en, &cap).and_then(|d| unsafe { d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }.ok()) {
                    // Windows sends the notifications only after the session list was asked for once
                    let _ = unsafe { mgr.GetSessionEnumerator() };
                    let n = l.notify.get_or_insert_with(|| SessNotify.into()).clone();
                    if let Err(e) = unsafe { mgr.RegisterSessionNotification(&n) } {
                        logf!("mic processing: RegisterSessionNotification {e}");
                    }
                    l.mgr = Some(mgr);
                }
            }
        }
        let who = self.listen.scan();
        // the recording end not found: nothing to watch — the processing keeps running
        let st = if self.listen.mgr.is_none() || who.is_some() { 2 } else { 1 };
        let old = STATE.swap(st, Ordering::Relaxed);
        if old != st {
            match &who {
                Some(w) => logf!("mic processing: {w} records from the cable"),
                None if st == 1 => logf!("mic processing: nobody records from the cable — asleep"),
                None => logf!("mic processing: the cable's recording end not found — no sleep"),
            }
            crate::router::wake();
        }
    }
}

impl Listen {
    /// The cable's recording sessions: the events go on the new ones; the app of the first active one
    /// (not this app).
    fn scan(&mut self) -> Option<String> {
        let mgr = self.mgr.clone()?;
        let ev = self.events.get_or_insert_with(|| SessEvents.into()).clone();
        let mut seen = HashSet::new();
        let mut pid_on = 0;
        unsafe {
            let se = mgr.GetSessionEnumerator().ok()?;
            for i in 0..se.GetCount().unwrap_or(0) {
                let Ok(ctl) = se.GetSession(i) else { continue };
                let Ok(c2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                let pid = c2.GetProcessId().unwrap_or(0);
                let Ok(p) = c2.GetSessionInstanceIdentifier() else { continue };
                let key = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const c_void));
                if !self.sessions.contains_key(&key) && ctl.RegisterAudioSessionNotification(&ev).is_ok() {
                    self.sessions.insert(key.clone(), ctl.clone());
                }
                seen.insert(key);
                if pid_on == 0 && pid != 0 && pid != *SELF_PID && ctl.GetState().is_ok_and(|s| s == AudioSessionStateActive) {
                    pid_on = pid;
                }
            }
            self.sessions.retain(|k, s| {
                let keep = seen.contains(k);
                if !keep {
                    let _ = s.UnregisterAudioSessionNotification(&ev);
                }
                keep
            });
        }
        (pid_on != 0).then(|| {
            let exe = appinfo::app_of(&util::exe_path_of(pid_on));
            if exe.is_empty() { format!("pid {pid_on}") } else { exe }
        })
    }
}

#[implement(IAudioSessionNotification)]
struct SessNotify;

impl IAudioSessionNotification_Impl for SessNotify_Impl {
    fn OnSessionCreated(&self, _session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        look();
        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
struct SessEvents;

impl IAudioSessionEvents_Impl for SessEvents_Impl {
    fn OnDisplayNameChanged(&self, _name: &PCWSTR, _ctx: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnIconPathChanged(&self, _path: &PCWSTR, _ctx: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnSimpleVolumeChanged(&self, _vol: f32, _mute: BOOL, _ctx: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnChannelVolumeChanged(&self, _n: u32, _vols: *const f32, _ch: u32, _ctx: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnGroupingParamChanged(&self, _group: *const GUID, _ctx: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnStateChanged(&self, _state: AudioSessionState) -> windows::core::Result<()> {
        look();
        Ok(())
    }

    fn OnSessionDisconnected(&self, _reason: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        look();
        Ok(())
    }
}
