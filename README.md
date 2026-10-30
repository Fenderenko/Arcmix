# Arcmix

[Українська](README.uk.md) · **English**

A small Windows tray app for headsets: a game / chat balance for any apps, an equalizer, microphone
processing and headset settings in one window next to the clock.

Arcmix started from the idea of Swiftlyx's [OpenChatMix](https://github.com/Swiftlyx/OpenChatMix); this repository is a fork of it.

<p align="center">
  <img src="docs/main.png?v=5" width="200" alt="Mix: output and microphone, game / chat balance, quick tiles and app groups">
  <img src="docs/equalizer.png?v=5" width="200" alt="Equalizer: EQ Windows with a preset and AutoEQ">
  <img src="docs/microphone.png?v=5" width="200" alt="Microphone processing: level, noise suppression, compressor, VST plugins">
  <img src="docs/settings.png?v=5" width="200" alt="Settings: devices, headset, hotkeys, appearance, general">
</p>

## Features

- **Game / chat balance for any app.** Put apps into Game / Chat / Media: drag one onto a group or add it with “+”; the trash button removes one (“+” brings it back). Every app keeps its own volume, and the balance (the dial on the headset, the slider in the window or hotkeys) scales those levels instead of overwriting them.
- **Auto-ducking.** While someone talks in a Chat app, the game (and optionally Media) gets quieter. Chat apps are listened to directly, so music or a video never triggers it.
- **Volume memory.** App volumes and the microphone level come back if Windows or another app resets them.
- **An equalizer for all sound, on any device.** *EQ Windows* is one curve for everything that plays on the output device — speakers, USB or Bluetooth headphones — with no cables and no delay, through Arcmix's own module in the Windows audio engine, which Arcmix puts on every output device by itself (one administrator prompt for all of them). A device's own effects (Windows', Realtek, Dolby…) keep working: the module runs them inside itself. See [The Arcmix module](#the-arcmix-module).
- **Equalizer presets.** Built-in presets, your own presets (edit, save, rename, delete) and the headset's own equalizer with the presets built into it, where it has them. With [Equalizer APO](https://sourceforge.net/projects/equalizerapo/) installed, Arcmix drives it too and adds AutoEQ correction for your headphone model.
- **Microphone.** Volume, mute hotkey, push-to-talk on a key or a mouse button, soft sound cues and a mute badge on the tray icon.
- **Hear yourself.** Record 5 seconds and play them back, or listen to your microphone live, after processing.
- **Microphone processing.** Neural noise suppression — light RNNoise, or NVIDIA's on RTX graphics cards (with the NVIDIA Audio Effects SDK installed), which also removes room echo, with adjustable strength — a compressor, a noise gate, a loudness ceiling and 64-bit VST2 / VST3 plugins, in an order you can change. Needs a virtual audio cable, e.g. the free [VB-CABLE](https://vb-audio.com/Cable/) or Virtual Audio Cable; a cable disabled in Windows is switched on when you pick it.
- **Bluetooth headphones battery** in the header, as Windows knows it.
- **Headset settings.** Battery with alerts and a time estimate, sidetone, auto power-off, mute light and more, depending on the model.
- **Output devices by priority.** Sound goes to the highest connected device in your list: headset on, to the headset; off, to the next one. The headset microphone can follow it. You choose what happens when a device connects (switch to it if it is higher in the list, always, or never) and when one disconnects.
- **Audio routing** *(experimental)*. Game, chat and media into separate virtual cables, for streaming or a separate equalizer per group.
- **One switch for the virtual cables.** Settings → Devices → *Virtual cables* turns routing and the processed microphone off together: sound and the microphone go direct, the settings stay.
- **On-screen hint.** Changing the balance or switching the output or input device with hotkeys shows a small hint on the screen.
- **Tray icon.** The app mark while the balance is equal; two capsules in the theme's game / chat colours once it moves — or the headset battery instead of the mark, or the battery as a second icon of its own (any device that reports it, Bluetooth headphones too). The mouse wheel over it changes the balance or shows the battery.
- **A compact window.** Four tabs — Mix, Sound, Microphone, Settings; settings open as pages, and the mouse back / forward buttons move through tabs and pages.
- **Features you can turn off.** Settings → General → Features: a feature that is off disappears from the window and does not run.
- **Updates in one click.** Settings → About → *Check for updates* asks GitHub for the latest release; *Update to …* downloads it, puts it in place of `Arcmix.exe` and restarts the app (settings and autostart stay). Nothing is checked on its own.
- **Portable settings.** Settings → Settings and help → *Settings next to Arcmix.exe* keeps `config.json` and the rest in the app folder instead of `%APPDATA%`.
- **Themes (Black hole by default, or your own colours), profiles, hotkeys, settings in a file, 9 languages** (English, Українська, Deutsch, Français, Español, Português (Brasil), Polski, Italiano, Türkçe).

## Download and install

1. Download `Arcmix-1.2.3.zip` from [Releases](../../releases) and unpack it to any folder (e.g. `C:\Programs\Arcmix`).
2. Run `Arcmix.exe`. The exe is not code-signed, so Windows SmartScreen may warn you: click **More info → Run anyway**.
3. The icon appears in the tray (click it to open the window). A short first-run guide walks through the basics, the equalizer module included.

Requirements: Windows 10 or 11, 64-bit, with the Microsoft Edge WebView2 Runtime (already part of Windows 11;
on Windows 10 it comes with Edge, or get it from [Microsoft](https://developer.microsoft.com/microsoft-edge/webview2/)).

Settings, the log and crash reports are kept in `%APPDATA%\Arcmix`, or next to `Arcmix.exe` when a `config.json`
lies there (*Settings next to Arcmix.exe*). **Settings → General → Start with Windows**
adds the app to autostart (the current user's `Run` key).

**Uninstall:** turn off *Start with Windows*, quit from the tray menu, delete the folder with `Arcmix.exe`
and, if you don't need the settings any more, `%APPDATA%\Arcmix`.

## Quick start

**Balance.** Open the window and put your apps into groups: the game into *Game*, Discord into *Chat*,
the browser or a player into *Media*. Drag an app onto a group tab, or click “+” next to *Apps* to pick one
of the open apps. Then turn the dial on the headset or move the slider. Default hotkeys:
`Ctrl+Shift+→` more game, `Ctrl+Shift+←` more chat, `Ctrl+Shift+↓` equal balance, `Ctrl+Alt+M` mute the microphone,
`Ctrl+Alt+→` / `Ctrl+Alt+←` next / previous output device, `Ctrl+Alt+↓` / `Ctrl+Alt+↑` next / previous input device.
They can be changed in **Settings → Hotkeys** (click an action, then press the new combination).

**Equalizer.** **Sound → Equalizer**: pick a preset or drag the curve. A changed curve can be saved as a new
preset; your presets can be overwritten, renamed and deleted. *EQ* with the headset's name is the headset's own
equalizer; *EQ Windows* is one curve for everything on the output device: the Arcmix module goes on every output
device by itself (the switch is in the first-run guide; Settings → Settings and help → *Introduction* runs it again).
On a headset with its own equalizer, turn on *EQ Windows on the headset* to add both curves.

**Microphone processing** (noise suppression, gate, VST):
1. Install [VB-CABLE](https://vb-audio.com/Cable/) and restart the PC.
2. **Microphone → Microphone processing**: turn on *Process microphone*, pick your real microphone and *Send to: CABLE Input*.
3. While processing is on, Arcmix makes **CABLE Output** Windows' default microphone: Discord, games and OBS that
   use the default microphone hear the processed voice (an app set to a particular microphone needs **CABLE Output**
   picked there). Picking a microphone in Arcmix then changes only what is processed; with processing off, Windows
   gets that microphone back. **Hear yourself** lets you check the voice. The cable adds about 45 ms of latency, noise suppression about 10 ms more.
   *Processing order* at the end of the chain changes the order of the steps.

**Audio routing** *(experimental)*: sends Game, Chat and Media into their own virtual cables, e.g. to capture
them separately in OBS. Each group needs a free cable. The cable used by microphone processing is never
routed, so one VB-CABLE is not enough for both: more cables come with
[VB-CABLE A+B](https://shop.vb-audio.com/en/audio-apps/12-vb-cable-ab.html) (VB-Audio donationware; it may not
be bundled with other apps, so get it from their shop).

**Output devices by priority.** **Settings → Devices → Output device priority → Change**: drag the devices into
your order. With *Switch sound automatically* on, the Windows output follows the highest connected device;
*A device connected* and *A device disconnected* set what happens then.

## The Arcmix module

The *All sound* equalizer runs as an effect inside the Windows audio engine, so it needs no virtual cables and adds
no delay. Arcmix puts it on every output device by itself, and on new ones as they appear, with one administrator
prompt for all devices present (the first-run guide has the switch). **Remove** on a device keeps it off there.

- **Windows asks for administrator permission** and sound stops for a second while the audio services restart.
- **What it changes:** it copies `ArcmixApo.dll` to `C:\Program Files\Arcmix`, registers it as an audio effect and adds
  it to the device's effects. Effects the device already has keep working inside the module. The module is not
  code-signed, so Arcmix also sets `DisableProtectedAudioDG = 1` (the audio engine then loads unsigned effects —
  Equalizer APO does the same).
- **Remove** puts the device's effects back as they were, and the protection too once no device has the module.
- **Needs** the device's *Audio enhancements* on in Windows; the window says when they are off.
- **Does not apply** to apps that use exclusive mode or ASIO, and to a Bluetooth headset in its hands-free mode.

## Headsets

The balance, ducking, equalizer presets, microphone tools, routing and themes work with any headset.
Battery, sidetone, the hardware equalizer and other headset settings need the headset's own protocol:

- **Arctis 7+, 7P+, 7X+** have a native driver and are tested with the real headset.
- **Arctis 7** is tested with the real headset too: battery, the ChatMix dial, the microphone button, sidetone,
  auto power-off, microphone volume and its 6-band hardware equalizer (checked against SteelSeries GG).
- **Two receivers at once** (say, an Arctis 7+ and an Arctis 7): Arcmix talks to both. The window follows the one
  whose sound plays, and a headset switched on is noticed at once, so output priority can move to it.
- Other Arctis models (Arctis 1, 5, 7, 9, Pro, Nova 3 / 4 / 5 / 7 / Pro / Elite, GameBuds and others) come from
  the device descriptions of [Linux-Arctis-Manager](https://github.com/elegos/Linux-Arctis-Manager) in
  `web/devices.json`. They are **not tested**: some settings may be missing or behave differently.
  Reports are welcome.
- **Hardware ChatMix** (Arctis 7, 9, Pro, Nova Pro — a separate "Chat" device in Windows): the headset's dial
  mixes its "Game" and "Chat" devices itself, so Arcmix sends the Chat apps to the "Chat" device on its own — the
  dial then works for Discord with nothing set in Discord (Settings → Devices → *Chat to the headset's “Chat”*).
  While the "Game" side plays, the "Chat" device is also Windows' communications device; an app that plays on
  "Chat" by its own setting joins the Chat group once, and a Chat app set to "Game" in its own settings gets a
  warning with what to choose there.
- **Arctis 7 (2018, 2019) and Arctis 9:** corrected after HeadsetControl and the Linux kernel driver (the USB
  interface, the ChatMix values, the Arctis 9 battery). They report nothing by themselves, so the dial is asked
  for four times a second, the microphone button every second and the power every three seconds.
- **Connection type:** Bluetooth and a 3.5 mm jack are shown as such; over USB Windows cannot tell a cable from
  a 2.4 GHz receiver, so a wireless model shows 2.4 GHz. A headset on a 3.5 mm jack sends no data, so no battery
  or dial position.
- **Nova 7 Gen 2, Nova 5, GameBuds, Nova Elite:** their parametric hardware equalizer is ready but off until someone
  checks it on a real headset. To test it, quit Arcmix, add `"untestedEq": true` to `%APPDATA%\Arcmix\config.json`
  and start it again; please report how it went.
- Without a supported headset, the window shows nothing about the headset (no battery, sidetone or
  headset equalizer); everything else works.

## Known limitations

- **A separate equalizer for game / chat / media** is only possible through audio routing with virtual cables:
  Windows captures an app's sound after its volume, so the original cannot be muted while a processed copy plays.
- **Bluetooth headsets:** when an app opens the headset microphone, Windows switches it to the Hands-Free mode
  (mono, lower quality). That is how Bluetooth works in Windows, not an Arcmix setting.
- **Vendor headset software** that creates its own virtual devices or changes per-app volume can fight with
  Arcmix. Close it or turn its mixer off.
- **Wireless headsets after a restart:** the receiver may report the headset as off until the headset is turned
  off and on again (seen with the Arctis 7+).
- **The Arcmix module** cannot add a separate curve per app: it works on the whole output device.
- **Hardware ChatMix** mixes two Windows devices inside the headset: everything on the "Game" device, Media too,
  follows the game side of the dial.
- **Auto power-off** is the headset's own timer: an Arctis 7 may turn off while quiet music plays (SteelSeries GG
  sees the same). Pick a longer time or *Never*.
- **Not tested live yet:** audio routing, push-to-talk and the microphone sound cues, the taskbar button,
  disabling a Windows device from the device list, headsets other than the Arctis 7+ family and the Arctis 7.
- VST plugins must be 64-bit.

## Troubleshooting

**Settings → Settings and help → Diagnostic report** puts a zip with the log, the crash report and the settings on the
desktop. Please attach it when you open an [issue](../../issues).

## Building from source

Requires Windows and [Rust](https://rustup.rs/) with the MSVC toolchain. Crate versions are pinned in `rust/Cargo.lock`.

```sh
cd rust/apo
cargo build --release
cd ..
cargo build --release
```

The audio-engine module (`rust/apo`) is built first: the app embeds it. The app is `rust/target/release/arcmix.exe`; `build.rs` embeds the icon (`assets/app.ico`) and the version from
`Cargo.toml`. Tests: `cargo test --release`.

Tools in `tools/` need Python 3:

- `mkicon.py`: the app icon, the mark on a dark tile (`assets/app.ico`, `assets/app.png`); `--svg` prints the starry
  vector icon used in About.
- `i18n_src.py`: the translations source, generates `web/i18n.json`; `check_i18n.py` reports missing strings.
- `gen_devices.py`: `web/devices.json` from Linux-Arctis-Manager's `device-configs`.
- `rnnoise_model.py`: `assets/rnnoise.bin` from the RNNoise sources.
- `mock/srv.py`: the same window with made-up data at http://127.0.0.1:8772/, for UI work without the app.

## Support

Arcmix is free. If it is useful to you, you can support it: [monobank jar](https://send.monobank.ua/jar/6VR4c6nxzz).

## Credits

Made by **Fenderenko**, written together with Claude Code (Anthropic). Arcmix stands on the work of:

- **Swiftlyx** — [OpenChatMix](https://github.com/Swiftlyx/OpenChatMix) (MIT): the idea of a game / chat mix
  for any headset and the first groundwork. Arcmix grew out of it.
- **elegos** and contributors — [Linux-Arctis-Manager](https://github.com/elegos/Linux-Arctis-Manager) (GPL-3.0):
  the device descriptions (`web/devices.json`), the description engine ported to Rust (`rust/src/devs/engine.rs`)
  and writing the equalizer to the headset memory once on exit.
- **Sapd** and contributors — [HeadsetControl](https://github.com/Sapd/HeadsetControl) (GPL-3.0): a reference for
  the Arctis 7+ commands.
- **File-New-Project** — [EarTrumpet](https://github.com/File-New-Project/EarTrumpet) (MIT): the layout of the
  Windows interface for an app's own output device.
- **Xiph.Org, Mozilla, Jean-Marc Valin** — [RNNoise](https://github.com/xiph/rnnoise) (BSD-3-Clause): noise
  suppression, ported to Rust together with its model.
- **loteran** — [Arctis Sound Manager](https://github.com/loteran/Arctis-Sound-Manager) (GPL-3.0): a reference for the
  parametric headset equalizer of Nova 7 Gen 2 and others.
- **Jaakko Pasanen** — [AutoEq](https://github.com/jaakkopasanen/AutoEq) (MIT): the headphone correction curves,
  downloaded when you pick a model.
- **Robert Bristow-Johnson** — the Audio EQ Cookbook: the equalizer filters.
- **Microsoft** — [windows-rs](https://github.com/microsoft/windows-rs) (MIT / Apache-2.0) and the WebView2 loader
  from the WebView2 SDK.
- **wravery** — [webview2-com](https://github.com/wravery/webview2-rs) (MIT).
- **dtolnay** and contributors — [serde, serde_json](https://serde.rs/) (MIT / Apache-2.0).
- **The Onest Project Authors** — the [Onest](https://github.com/googlefonts/onest) font (SIL OFL 1.1).
- **Lucide Contributors** — the [Lucide](https://lucide.dev) icons of the window (ISC).
- **Steinberg** — the [VST 3 SDK](https://github.com/steinbergmedia/vst3_pluginterfaces) interfaces (MIT).
  VST is a registered trademark of Steinberg Media Technologies GmbH.
- **VB-Audio** — [VB-CABLE](https://vb-audio.com/Cable/), the virtual cable the microphone processing and routing
  rely on. It is not included with Arcmix.

License texts and notices: [THIRD_PARTY.md](THIRD_PARTY.md).

## License

Arcmix is licensed under the **GNU GPL v3** — see [LICENSE](LICENSE). It is GPL because it includes the device
descriptions and engine from Linux-Arctis-Manager (GPL-3.0).
