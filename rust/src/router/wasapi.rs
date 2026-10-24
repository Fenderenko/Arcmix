//! A minimal WASAPI wrapper for routing: capture (from a microphone or as a loopback of an output
//! device — this is how virtual cables are read) and output. All calls — from a thread with COM
//! (MTA) initialized.

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Media::Audio::{AUDCLNT_SHAREMODE_SHARED, IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, WAVEFORMATEX},
        System::{
            Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
            Threading::{AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, WaitForSingleObject},
        },
    },
    core::{HSTRING, w},
};

use crate::dsp::{SampleFmt, scratch};

const STREAM_LOOPBACK: u32 = 0x0002_0000;
const STREAM_EVENT: u32 = 0x0004_0000;
const STREAM_NO_PERSIST: u32 = 0x0008_0000;
const BUF_SILENT: u32 = 0x2;

pub fn hr_err(what: &str, e: &windows::core::Error) -> String {
    format!("{what}: 0x{:08x}", e.code().0 as u32)
}

/// A clear explanation of typical device errors (Ukrainian text — translated by the window).
pub fn err_hint(err: &str) -> &'static str {
    if err.contains("0x8889000a") {
        "Пристрій зайнятий: його вже використовує інша програма або той самий кабель."
    } else if err.contains("device not found") || err.contains("0x88890004") {
        "Пристрій не знайдено — можливо, його від'єднано або вимкнено."
    } else if err.contains("feedback") {
        "Вивід не може бути тим самим пристроєм, що й вхід."
    } else if err.contains("mic input is a virtual cable") {
        "Мікрофоном обрано віртуальний кабель — оберіть справжній мікрофон."
    } else if err.contains("already used by routing") {
        "Цей кабель уже використовує маршрутизація — оберіть інший."
    } else if err.contains("unsupported device format") || err.contains("0x88890008") {
        "Непідтримуваний формат пристрою."
    } else {
        ""
    }
}

/// Reads WAVEFORMATEX / WAVEFORMATEXTENSIBLE; false — a format the app cannot handle.
fn parse_wave_format(p: *const WAVEFORMATEX) -> (SampleFmt, bool) {
    let b = unsafe { std::slice::from_raw_parts(p as *const u8, 18) };
    let u16at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let tag = u16at(0);
    let mut f = SampleFmt {
        channels: u16at(2) as usize,
        rate: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        block: u16at(12) as usize,
        bits: u16at(14) as u32,
        float: false,
    };
    let cb = u16at(16);
    match tag {
        3 => f.float = true,
        1 => {}
        0xFFFE => {
            if cb < 22 {
                return (f, false);
            }
            let x = unsafe { std::slice::from_raw_parts(p as *const u8, 40) };
            let valid = u16::from_le_bytes([x[18], x[19]]);
            if valid > 0 {
                f.bits = valid as u32;
            }
            match u32::from_le_bytes([x[24], x[25], x[26], x[27]]) {
                3 => f.float = true,
                1 => {}
                _ => return (f, false),
            }
        }
        _ => return (f, false),
    }
    if f.float && f.bits != 32 {
        return (f, false);
    }
    let ok = f.channels > 0 && f.channels <= 16 && f.rate >= 8000 && f.block > 0;
    (f, ok)
}

pub struct Client {
    pub dev: IMMDevice,
    client: IAudioClient,
    rc: Option<IAudioRenderClient>,
    cc: Option<IAudioCaptureClient>,
    event: HANDLE,
    pub fmt: SampleFmt,
    pub buf_frames: u32,
    /// the Windows audio engine period, frames
    pub period: u32,
}

/// Opens a device by id. render — output; loopback — capture of what plays on an output device;
/// otherwise — normal recording (a microphone).
pub fn open(en: &IMMDeviceEnumerator, id: &str, render: bool, loopback: bool) -> Result<Client, String> {
    let dev = unsafe { en.GetDevice(&HSTRING::from(id)) }.map_err(|e| hr_err("device not found", &e))?;
    let client: IAudioClient = unsafe { dev.Activate(CLSCTX_ALL, None) }.map_err(|e| hr_err("Activate", &e))?;
    let wfx = unsafe { client.GetMixFormat() }.map_err(|e| hr_err("GetMixFormat", &e))?;
    if wfx.is_null() {
        return Err("GetMixFormat: null".into());
    }
    struct Free(*mut WAVEFORMATEX);
    impl Drop for Free {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0 as *const _)) };
        }
    }
    let _free = Free(wfx);
    let (f, ok) = parse_wave_format(wfx);
    if !ok {
        return Err(format!("unsupported device format ({} ch, {} Hz, {} bit)", f.channels, f.rate, f.bits));
    }
    let mut flags = STREAM_NO_PERSIST;
    let mut dur = 200_000i64; // 20 ms (100 ns units)
    // output and a microphone wake their thread by event when data is due; loopback capture is
    // polled (its events are not reliable on every Windows 10)
    if render {
        flags |= STREAM_EVENT;
    } else {
        dur = 400_000; // more slack for capture
        if loopback {
            flags |= STREAM_LOOPBACK;
        } else {
            flags |= STREAM_EVENT;
        }
    }
    unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, dur, 0, wfx, None) }.map_err(|e| hr_err("Initialize", &e))?;
    let buf_frames = unsafe { client.GetBufferSize() }.unwrap_or(0);
    let (mut def_period, mut min_period) = (0i64, 0i64);
    let mut period = 0u32;
    if unsafe { client.GetDevicePeriod(Some(&mut def_period), Some(&mut min_period)) }.is_ok() && def_period > 0 {
        period = (def_period * f.rate as i64 / 10_000_000) as u32;
    }
    if period == 0 || period > buf_frames {
        period = buf_frames / 2;
    }
    let mut c = Client { dev, client, rc: None, cc: None, event: HANDLE::default(), fmt: f, buf_frames, period };
    if render || !loopback {
        c.event = unsafe { CreateEventW(None, false, false, None) }.map_err(|e| hr_err("CreateEvent", &e))?;
        unsafe { c.client.SetEventHandle(c.event) }.map_err(|e| hr_err("SetEventHandle", &e))?;
    }
    if render {
        c.rc = Some(unsafe { c.client.GetService() }.map_err(|e| hr_err("GetService", &e))?);
    } else {
        c.cc = Some(unsafe { c.client.GetService() }.map_err(|e| hr_err("GetService", &e))?);
    }
    Ok(c)
}

impl Client {
    /// prefill — frames of silence put ahead for output (0 — the whole buffer).
    pub fn start(&self, mut prefill: u32) -> Result<(), String> {
        if let Some(rc) = &self.rc {
            // a bit of silence before the start so the engine has something to play
            if prefill == 0 || prefill > self.buf_frames {
                prefill = self.buf_frames;
            }
            unsafe {
                if rc.GetBuffer(prefill).is_ok() {
                    let _ = rc.ReleaseBuffer(prefill, BUF_SILENT);
                }
            }
        }
        unsafe { self.client.Start() }.map_err(|e| hr_err("Start", &e))
    }

    pub fn padding(&self) -> windows::core::Result<u32> {
        unsafe { self.client.GetCurrentPadding() }
    }

    /// The stream wakes its thread by event (output, a microphone), not by polling.
    pub fn evented(&self) -> bool {
        !self.event.is_invalid()
    }

    /// Waits for the stream event (up to ms).
    pub fn wait(&self, ms: u32) {
        unsafe {
            WaitForSingleObject(self.event, ms);
        }
    }

    /// Writes n frames that fill produces (f32, the device channels).
    pub fn render_frames(&self, n: u32, tmp: &mut Vec<f32>, fill: impl FnOnce(&mut [f32], usize)) -> windows::core::Result<()> {
        let Some(rc) = &self.rc else { return Ok(()) };
        let data = unsafe { rc.GetBuffer(n) }?;
        let buf = scratch(tmp, n as usize * self.fmt.channels);
        fill(buf, n as usize);
        let dst = unsafe { std::slice::from_raw_parts_mut(data, n as usize * self.fmt.block) };
        self.fmt.from_float(dst, buf, n as usize);
        unsafe { rc.ReleaseBuffer(n, 0) }
    }

    /// Reads all ready packets; f gets f32 (the device channels) and the frame count.
    pub fn capture_all(&self, tmp: &mut Vec<f32>, mut f: impl FnMut(&mut [f32], usize)) -> windows::core::Result<()> {
        let Some(cc) = &self.cc else { return Ok(()) };
        loop {
            if unsafe { cc.GetNextPacketSize() }? == 0 {
                return Ok(());
            }
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut::<u8>(), 0u32, 0u32);
            unsafe { cc.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }?;
            if frames > 0 {
                let n = frames as usize * self.fmt.channels;
                let buf = scratch(tmp, n);
                if flags & BUF_SILENT != 0 || data.is_null() {
                    buf.fill(0.0);
                } else {
                    let src = unsafe { std::slice::from_raw_parts(data, frames as usize * self.fmt.block) };
                    self.fmt.to_float(buf, src, frames as usize);
                }
                f(buf, frames as usize);
            }
            unsafe { cc.ReleaseBuffer(frames) }?;
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            if !self.event.is_invalid() {
                let _ = CloseHandle(self.event);
            }
        }
    }
}

/// A thread for sound: COM (MTA) and the "Pro Audio" priority. Dropped last on the thread (declare
/// it first) so all COM objects are gone before COM is uninitialized.
pub struct AudioThread(Option<HANDLE>);

pub fn audio_thread() -> AudioThread {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut idx = 0u32;
        AudioThread(AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut idx).ok())
    }
}

impl Drop for AudioThread {
    fn drop(&mut self) {
        unsafe {
            if let Some(h) = self.0 {
                let _ = AvRevertMmThreadCharacteristics(h);
            }
            CoUninitialize();
        }
    }
}
