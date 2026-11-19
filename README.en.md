<img src="assets/brand/logo-light.svg" alt="Kite Island" height="64">

# Kite Island

[Русский](README.md) · **English**

A dynamic island for Windows 10 and 11: a dark pill centered at the top edge of the screen. Hovering expands it
into a panel with seven built-in tabs (plus custom tabs, a notification center, and settings). Rendered on the GPU
(Direct2D over DirectComposition), it uses virtually zero CPU when idle, has an ~2.5 MB executable (≤ 3 MB), and
uses up to ~55 MB peak memory in Task Manager (depending on open tabs and icon cache).

## Requirements

- Windows 10 (version 2004, build 19041) or Windows 11, x64
- To build from source: Rust (stable, MSVC) and the Windows SDK

## Installation

Download one of the release files from [GitHub Releases](https://github.com/GulovM/island/releases/latest) or via the [Kite Island homepage](https://gulovm.github.io/island/):

- **`island-<version>-setup.exe`** — installer. Installs into `%LOCALAPPDATA%\Programs\Island` without administrator
  rights, and optionally adds autostart and a desktop shortcut. Uninstallable via "Settings → Apps", where it offers
  to delete settings and cached data.
- **`island-<version>-portable.zip`** — no installation required: extract anywhere and run `island.exe`. Only a tray
  icon and the settings folder `%APPDATA%\island` are created on your system.

Checksums are provided in `SHA256SUMS.txt`. The executable is not digitally signed, so Windows SmartScreen may show a
warning ("More info → Run anyway").

Clipboard history and Quick Access are stored in `%APPDATA%\island` encrypted with Windows DPAPI (bound to your
Windows user account); copying the folder to another computer or user account will not decrypt them.

Silent installation: `island-<version>-setup.exe /VERYSILENT /TASKS=autostart`.

To exit the app: right-click the tray icon → "Exit".

## How to Use

- **Hover the cursor at the top edge of the screen** — the island expands; move the cursor away — it collapses.
- The collapsed island shows configurable side icons (by default, keyboard layout on the left and Wi-Fi on the right;
  you can pick layout, Wi-Fi / airplane mode, Bluetooth, battery, or none) and the center clock (which can be hidden
  in Settings → System while keeping side icons; when a timer or Pomodoro is running, a countdown pill appears with
  phase color coding: orange for standard timer, red for focus, green for break). When music is playing, the notch
  turns into a **mini-player** (album art, track title, artist, track progress, and timer bar, periodically and on
  layout switch peeking the clock/icons; can be turned off in Settings → System to keep the regular notch), while
  live WASAPI stereo audio waves animate along the left and right edges. Volume, timer, and charging show brief
  toasts, while app notifications and calendar reminders pop up in a capsule below the notch. Clicking a popup
  notification opens or focuses the sending app (raising messengers/social apps from tray) and marks it as read;
  right-clicking dismisses the capsule.
- **Main tabs:** Controls (Wi-Fi, airplane mode, Bluetooth, Do Not Disturb from the widget tray, audio volume & output device, night light, dark theme, energy saver, battery, media player), Today (clock with 6 faces, weather, calendar with custom events, repeats, reminders, two-way sync with Google Calendar and iCloud plus ICS links, timer and Pomodoro mode with focus/break cycles, tasks with priority levels, sorting, and draft preservation when the island closes),
  Stats (CPU, GPU, memory, disks, uptime, network, and speed test), Clipboard (Quick Access and clipboard history), Apps (app launcher and quick links),
  Wallpaper (wallpaper switcher), Power (lock, sleep, restart, shut down).
- **Notifications:** the **bell** icon next to the gear opens the Windows Notification Center (`UserNotificationListener`)
  and calendar reminders with an unread counter, color emoji support, `Inter` typography (falling back to the system
  UI font) with `Semi Bold` highlighting for the notification title in the `App · Title · time` header line,
  a "Mark all read" action and read-state sync with Windows Notification Center, plus per-app rules (right-click a card
  or open the "Apps" list: normal, silent, or disabled). While popup capsules are enabled, native Windows banners are
  automatically suppressed to avoid duplicates and automatically pause when Windows Do Not Disturb is active.
- **Widget customization:** the pencil button on the right of the tab bar (or long-pressing empty space) opens grid
  edit mode on Controls, Today, and Stats. Widgets can be dragged, hidden into the tray (`−`), and resized. Clicking
  "Done" automatically shrinks the island to fit visible widgets; re-entering edit mode highlights edge handles so you
  can resize the island back out.
- **Clipboard & Quick Access** (Clipboard tab): on the left is **Quick Access**, persistent storage for up to 30 items —
  files (only the path is stored; missing files are marked "not found"), links, text snippets, and images; on the right
  is **Clipboard** history, which also automatically captures files copied in Windows Explorer (`CF_HDROP`), keeps items
  for 24 hours, and cleans up automatically.
  - **External drag-and-drop onto the island:** as soon as you start dragging a file, link, or text anywhere in Windows,
    the island immediately displays a "Add to Quick Access — Drag onto the island" hint; hovering over the island
    (including strip mode) changes it to "Quick Access — Release to add".
  - **Multi-selection & drag-out:** both Quick Access and Clipboard history support multi-selection (`Ctrl+Click` to
    toggle individual items, `Shift+Click` for ranges) with batch copy (`Ctrl+C`), pin (`Ctrl+P`), and delete (`Del`),
    plus dragging one or multiple selected items out into Explorer, messengers, or editors.
  - **Copying & opening:** `Ctrl+C` (including files of any format via `CF_HDROP` and images as `CF_DIB` + PNG file)
    places the selected entries onto the Windows clipboard for direct pasting into Explorer folders, chats, and editors;
    `Enter` opens files and links in Quick Access. After copying, opening, or completing a drag-out, the island
    automatically collapses and returns focus to the previous window.
- **Hotkeys & focus management:** `Ctrl+Alt+Space` opens the Apps tab with search (configurable in `config.toml`), and
  Quick Access has its own hotkey button ("Set hotkey") in the top-right corner of the section. When opened via hotkey,
  the island stays open until your cursor enters and leaves it; pressing the same hotkey again, pressing `Esc` (or
  exiting grid edit mode), or clicking into another window collapses the island and restores focus to the previous
  window. In the expanded island, `←` / `→` arrow keys cycle through tabs (and return from Settings or Notifications
  back to the current tab), and text input fields support `Ctrl+←` / `Ctrl+→` (plus `Shift` to select) for word-by-word
  cursor navigation.
- Right-clicking the expanded island toggles "strip" mode: over maximized windows, the island collapses to a thin line.
- In fullscreen games (DirectX), the island hides automatically.

## Glass Effect (iOS 26 Liquid Glass)

The expanded island uses an iOS 26 liquid glass look with background blur, luminous border, and highlights. In Settings → Appearance:
- **Glass** (`glass`, default `1`) — enables translucent glass; `0` renders a solid body.
- **Opacity** (`glass_opacity`, 0–100, default `70`, in `Windows` style) — body translucency level of the expanded island.
- **Material** (`material`, default `regular`, in `macOS` style) — five density presets: `Ultrathin`, `Thin`, `Regular`, `Thick`, `Chrome`.
- **Color style** (`palette_style`, default `windows`) — Windows system colors or iOS 26 palette ("macOS" in Settings; legacy `ios` is also accepted).

If Windows Transparency Effects are disabled, blur is omitted and the body becomes denser. The expanded island always remains visible in screen recordings, streams (OBS), and screenshots; the screenshot-hide setting hides only the collapsed notch.

## Settings

Click the **gear** icon on the right of the tab bar. Top sections are **Appearance**, **Tabs**, and **System**; the selected section is remembered until exit.

- **Glass**, **Opacity**, **Material**, **Color style**, **Accent pill**, and **UI scale** — iOS 26 glass options, palette style (`Windows` or `macOS`; "Opacity" is shown in Windows style and "Material" in macOS style), active tab pill tint, and overall UI scale.
- **System colors** — derives accent and background from Windows colors (follows Wallpaper Engine / Windows accent changes).
- **Colors** — full color picker for accent or background with saturation/brightness square, hue slider, `#RRGGBB` input, and six built-in themes. Text and card contrast automatically adapt to background luminance via APCA.
- **Corner radius** — slider with live highlight preview while dragging.
- **Tabs** — enable/disable tabs (up to 8 active at once), reorder via **mouse drag-and-drop**, and create/rename/delete custom tabs.
- **System** — **autostart**, **clock and custom side icons** on the collapsed island (left/right slot), **collapsed island mode** ("Auto" or "Always strip"), **collapsed mini-player**, **strip** over maximized windows, daily completed-task cleanup, free widget layout, hide collapsed island on screenshots, **language**, **popup notifications**, and **updates** (manual check & auto-update).
- "Reset appearance" restores defaults.

Changes take effect immediately and are saved to `%APPDATA%\island\config.toml`.

## config.toml

Created on first launch with inline comments. Key configuration options:

| Key | Description |
|---|---|
| `glass` | glass effect: `1` enabled, `0` solid body |
| `glass_opacity` | expanded island glass opacity in Windows style, `0`–`100` (default `70`) |
| `material` | glass material in macOS style: `ultrathin`, `thin`, `regular` (default), `thick`, `chrome` |
| `palette_style` | palette style: `windows` (default) or `macos` (legacy `ios` also accepted) |
| `accent_pill` | `1` — accent-tinted active tab pill, `0` — neutral |
| `system_colors`, `accent`, `bg`, `corner_radius` | Windows colors or custom `#RRGGBB` hex colors and bottom corner radius |
| `tabs`, `hidden_tabs`, `custom_tabs` | tab order, hidden tabs, and custom tabs (`id\|name\|layout`) |
| `idle_face`, `collapsed_time`, `collapsed_left`, `collapsed_right` | collapsed face visibility, separate clock toggle (`1`/`0`), and left/right slot icons (`layout`, `wifi`, `bluetooth`, `battery`, `none`) |
| `collapsed_mode` | `auto` (notch, or thin strip over maximized windows) or `strip` (always thin strip) |
| `strip_over_maximized` | `1` — shrink to a thin strip over maximized windows so browser tabs stay unobstructed |
| `live_media`, `collapsed_player`, `live_toasts`, `popup_notifications` | notch media indicators (mini-player & edge waves), separate collapsed mini-player toggle, brief toasts (timer, volume, charging), and popup notification capsules (`1` on, `0` off) |
| `todo_clean_done`, `free_layout`, `hide_on_screenshot` | auto-clear completed tasks on new day, free widget grid layout, hide collapsed island on screenshots (`1`/`0`; expanded island stays visible) |
| `leave_delay_ms`, `env_poll_ms` | collapse delay (ms) and fullscreen environment poll interval (ms) |
| `collapsed_width`, `collapsed_height` | collapsed island dimensions (logical pixels) |
| `hotkey` | global hotkey for the Apps tab (`""` to disable) |
| `quick_hotkey` | global hotkey for Quick Access (unset by default; normally assigned from the Quick Access UI) |
| `wallpaper_dir` | wallpaper folder for the Wallpaper tab |
| `weather_city`, `clock_style` | city for the weather widget (empty = IP lookup via ipwho.is) and clock face (`0`–`5`) |
| `calendar_ics`, `layout_controls`, `layout_today`, `layout_stats` | ICS calendar feed URLs and saved widget layouts (`v2`); Google and iCloud are connected inside Today |
| `ui_scale` | multiplier over Windows DPI scale, `0.9`–`2.0` (default `1.2`) |
| `language`, `auto_update` | UI language (`auto`, `ru`, `en`) and automatic update installation (`0`/`1`) |
| `[snippets]` | quick web links in the Apps tab: `name = "url"` |

You can edit the file manually: tray menu → "Open config.toml", then "Reload config".

## Building from Source

```powershell
cargo build --release          # island\target\release\island.exe
cargo test                     # unit tests
powershell -ExecutionPolicy Bypass -File tools\package.ps1   # portable zip, installer, and SHA256SUMS.txt in dist\
```

The installer uses [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`); without it,
`package.ps1` builds only the portable zip. Installer script: `installer\island.iss`. Release workflow: [CONTRIBUTING.md](CONTRIBUTING.md).

When the Windows SDK (`rc.exe`) is installed, the icon and version metadata are embedded in the exe; without it, the build succeeds without the embedded icon.

## Troubleshooting

- Log file: `%APPDATA%\island\island.log`.
- "Failed to register hotkey" — the shortcut is already taken by another app; change `hotkey` in `config.toml`.
- Island doesn't open on top-edge hover while strip mode is active (dot on the left of the tab row) — right-click the expanded island to toggle strip mode off.

## Project Origin

Kite Island is an independent implementation inspired by **Hitodama** (Mainak Deb Das, Otism). All code was written
from scratch; the original's behavior was studied solely through observation (reverse engineering), and **no source
code or assets were copied**. The project is not affiliated with the authors of Hitodama and is **non-commercial**:
free, with no ads and no data collection. See [NOTICE.md](NOTICE.md) for details.

## Copyright and License

**Copyright © 2026 GulovM.** Distributed under the non-commercial brand-protection license — [LICENSE](LICENSE) (see also [NOTICE.md](NOTICE.md) and [TERMS.md](TERMS.md)):

- **Permitted:** You may freely download, install, clone, study, and modify the project for personal, educational, and non-commercial purposes.
- **No commercial use or sale:** Selling the application or derivative works, paid distribution, monetization, or commercial use is prohibited without prior written consent from the author.
- **Mandatory fork attribution:** Any fork or derivative project based on this software must retain `LICENSE` and `NOTICE.md` and clearly credit **Kite Island** by **GulovM** (`https://github.com/GulovM/island`) as its base.
- **Brand name & logo restriction:** Third-party forks and derivative applications must NOT use **"Kite Island"** in their own application/product name (except to factually credit "based on Kite Island by GulovM") and must NOT use the official Kite Island logos or icons (`assets/brand/*`, `assets/island.ico`, `favicon.svg`).

Links: [Privacy Policy](PRIVACY.md) · [Terms of Service](TERMS.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md).
