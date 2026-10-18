# Third-party code and notices

Arcmix is licensed under the GNU GPL v3 (see [LICENSE](LICENSE)). It includes, or is built from, the works below.

## Linux-Arctis-Manager — GPL-3.0

[Linux-Arctis-Manager](https://github.com/elegos/Linux-Arctis-Manager) by elegos and contributors.

- `web/devices.json` is generated from its `device-configs` by `tools/gen_devices.py`.
- `rust/src/devs/engine.rs` and `rust/src/devs/builtins.rs` port part of its device description engine to Rust.
- Writing the equalizer curve to the headset memory once on exit follows its approach.

Licensed under the GNU GPL v3, the same text as [LICENSE](LICENSE).

## RNNoise — BSD-3-Clause

`rust/src/rnnoise.rs` is a Rust port of [RNNoise](https://github.com/xiph/rnnoise) (classic version, commit 7f449bf),
and `assets/rnnoise.bin` contains its built-in model weights (extracted with `tools/rnnoise_model.py`).
The port also includes parts of the Opus codec code bundled with RNNoise (KISS FFT, pitch analysis, LPC).

```
Copyright (c) 2017, Mozilla
Copyright (c) 2007-2017, Jean-Marc Valin
Copyright (c) 2005-2017, Xiph.Org Foundation
Copyright (c) 2003-2004, Mark Borgerding
Copyright (c) 2007-2008 CSIRO
Copyright (c) 2008-2011 Octasic Inc.
Copyright (c) 2018 Gregor Richards

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of the Xiph.Org Foundation nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Onest font — SIL OFL 1.1

`web/onest.ttf` (embedded into `Arcmix.exe`) is the [Onest](https://github.com/googlefonts/onest) font,
Copyright 2021 The Onest Project Authors. The license text is in [web/OFL-Onest.txt](web/OFL-Onest.txt).

## Rust crates

Compiled into `Arcmix.exe` from crates.io (exact versions in `rust/Cargo.lock`):

| Crates | License | Copyright |
|---|---|---|
| `windows`, `windows-core`, `windows-collections`, `windows-future`, `windows-link`, `windows-numerics`, `windows-result`, `windows-strings`, `windows-threading` | MIT or Apache-2.0 | Microsoft Corporation |
| `webview2-com`, `webview2-com-sys` | MIT | the [webview2-rs](https://github.com/wravery/webview2-rs) authors |
| `serde`, `serde_core`, `serde_json`, `itoa`, `zmij`, `thiserror` | MIT or Apache-2.0 (`zmij`: MIT) | their authors (David Tolnay and contributors) |
| `memchr` | MIT or Unlicense | Copyright (c) 2015 Andrew Gallant |

Arcmix uses them under the MIT license:

```
MIT License

Copyright (c) Microsoft Corporation.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

The other MIT crates use the same text with their own copyright holders listed above.

Build-time only (procedural macros, not part of the exe): `serde_derive`, `thiserror-impl`, `webview2-com-macros`,
`windows-implement`, `windows-interface`, `syn`, `quote`, `proc-macro2`, `unicode-ident`.

## WebView2 loader

`webview2-com-sys` links `WebView2LoaderStatic.lib` from the Microsoft Edge WebView2 SDK
(NuGet package [Microsoft.Web.WebView2](https://www.nuget.org/packages/Microsoft.Web.WebView2)) statically,
© Microsoft Corporation, under that SDK's license. The WebView2 Runtime itself is part of Windows and is not included.

## Interfaces and references (no code copied)

- **VST 3 SDK** ([pluginterfaces](https://github.com/steinbergmedia/vst3_pluginterfaces), MIT, Steinberg Media
  Technologies GmbH): structure layouts and method order in `rust/src/vst/vst3.rs` follow its headers.
  VST is a registered trademark of Steinberg Media Technologies GmbH.
- **EarTrumpet** ([File-New-Project/EarTrumpet](https://github.com/File-New-Project/EarTrumpet), MIT): the layout
  of the Windows interface for an app's own output device (`rust/src/audio/com.rs`).
- **HeadsetControl** ([Sapd/HeadsetControl](https://github.com/Sapd/HeadsetControl), GPL-3.0): a reference for the
  Arctis 7+ commands (`rust/src/devs/hid.rs`).
- **Audio EQ Cookbook** by Robert Bristow-Johnson: the peaking filter formulas (`rust/src/dsp.rs`).
- **OpenChatMix** ([Swiftlyx/OpenChatMix](https://github.com/Swiftlyx/OpenChatMix), MIT): the idea and the first
  groundwork Arcmix grew out of.

## Not included

[VB-CABLE](https://vb-audio.com/Cable/) and [VB-CABLE A+B](https://shop.vb-audio.com/en/audio-apps/12-vb-cable-ab.html)
by VB-Audio are not bundled; the app only links to their pages. Install them from VB-Audio.
