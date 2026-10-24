Arcmix {version}
{rule}

A Windows tray app for headsets: a game / chat balance for any apps, an equalizer,
microphone processing and headset settings.

Start
-----
1. Unpack this folder anywhere, e.g. C:\Programs\Arcmix.
2. Run Arcmix.exe. It is not code-signed: if Windows SmartScreen warns you,
   click "More info" -> "Run anyway".
3. The icon appears in the tray; click it to open the window.

Needs Windows 10 or 11 (64-bit) with the Microsoft Edge WebView2 Runtime
(already part of Windows 11).

Settings, the log and crash reports: %APPDATA%\Arcmix
Start with Windows: Settings -> General -> Start with Windows.
Uninstall: turn that off, quit from the tray menu, delete this folder and %APPDATA%\Arcmix.

The "All sound" equalizer runs as Arcmix's own module in the Windows audio engine:
Sound -> Equalizer -> All sound -> Install (Windows asks for administrator permission).
Remove puts everything back.

Microphone processing and audio routing need a virtual audio cable,
e.g. the free VB-CABLE: https://vb-audio.com/Cable/

The guide, known limitations, the source code and issues:
https://github.com/Fenderenko/Arcmix

Українською
-----------
Розпакуйте теку будь-куди й запустіть Arcmix.exe. Якщо Windows SmartScreen попередить —
«Докладніше» -> «Усе одно запустити». Значок з'явиться в треї. Мову можна змінити в
Налаштування -> Загальне -> Мова. Посібник українською — README.uk.md на GitHub.

Support Arcmix: https://send.monobank.ua/jar/6VR4c6nxzz

Arcmix grew out of the idea of Swiftlyx's OpenChatMix: https://github.com/Swiftlyx/OpenChatMix
Made by Fenderenko. License: GNU GPL v3 (LICENSE.txt). Third-party notices: THIRD_PARTY.md,
OFL-Onest.txt (the Onest font built into the app).
