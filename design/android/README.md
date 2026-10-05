# Pi in your pocket

**Android screens for Pi, built as an app in [crates/pi_android](../../crates/pi_android/README.md) on the [GPUI Android backend](../../crates/gpui_android/README.md). The app connects to a computer over SSH and follows its durable sessions; sample sessions are there to look around without one.**

**[Open the gallery](index.html)** · [Overview PNG](overview.png)

## The premise

Pi does not run on the phone. It runs on your computer or a server, as it does over SSH today: the remote helper keeps sessions working when the desktop disconnects, and the phone becomes another way in. The phone is for the moments away from the desk:

- seeing what is running and what needs you,
- answering Pi's questions and permission requests,
- following a run as it happens,
- reviewing what changed and asking for a revision,
- starting a task, with a file, screenshot or log attached.

Editing code, terminals and long configuration stay on the desktop.

## Twelve screens

| Screen | What it shows |
| --- | --- |
| [01 · Connect](screens/01-connect.png) | First run: “Where does Pi run?” A computer address, this phone's SSH key (generated on the phone, private key kept in Android's keystore) and what connecting does. Pairing by QR code from Pi Desktop is the shortcut. |
| [02 · Sessions](screens/02-sessions.png) | Home, sorted by what you can do: **Needs you** first (with an Answer button), then **Working**, then today's finished, stopped and failed sessions. The connected computer is always named. New session sits in the thumb zone. |
| [03 · Starting](screens/03-starting.png) | The desktop's landing on a phone: “What should we change?”, the composer itself, and starting points that fill the draft but never send it. The project and computer are one chip. |
| [04 · Working](screens/04-working.png) | The run rail as soft tiles: done stages tinted, the live stage outlined with its file diff, the stages ahead dashed. Status and Stop sit together at the bottom, with Queue a follow-up below. |
| [05 · Waiting for you](screens/05-waiting.png) | A permission request as a bottom sheet with the exact command, where it runs, and explicit choices. Picking a row does not answer; **Answer** does, and **Later** leaves it waiting. |
| [06 · Done](screens/06-done.png) | Pi's closing words first, the stage tiles collapsed into one line, then the changed files and check, and one primary action: Review. |
| [07 · Review](screens/07-review.png) | One file at a time, chosen from chips; a unified diff that fits a phone's width; tap lines to attach them to a revision request typed right below. These are observed edits already on the computer: there is no Accept. |
| [08 · Typing](screens/08-typing.png) | The composer above the keyboard: a pasted screenshot and an attached log as removable chips, `@` suggestions in a strip attached to the composer, the model and thinking level one tap away. |
| [09 · Details](screens/09-details.png) | The desktop inspector as a bottom sheet, closed until asked for: context use, cost, observed edits, then disclosures. |
| [10 · Notifications](screens/10-alerts.png) | How the phone is usually reached: a question with its answer buttons, a finished session with Review, and one quiet ongoing notification while sessions work. |
| [11 · Evening](screens/11-evening.png) | Screen 04 in the Evening theme. The phone follows the system's dark mode unless a theme is chosen. |
| [12 · Settings and tools](screens/12-settings.png) | Computers, appearance, notifications and typing on the phone; models and resources belong to the computer and open its settings. |

## Phone contract

- **Thumb zone:** the action that moves work forward (Answer, Review, Send, New session) is at the bottom. Navigation and rarely used actions are at the top.
- **Touch targets:** at least 48 dp, with icons drawn at 20–24 dp inside them. Nothing depends on hover.
- **Sheets, not windows:** details, questions and pickers are bottom sheets over a scrim. Back closes the sheet before it leaves the screen, and a dragged-down sheet changes nothing.
- **Keyboard-aware:** the composer rides on the keyboard's inset, sliding with it; the conversation stays readable above. Return adds a line by default; the send button sends.
- **Inspecting is not applying:** as on the desktop, choosing a file, model or command changes nothing until an explicit action. A revision is a new prompt with the selected lines attached.
- **Honest state:** Working, Needs you, Done, Stopped and Failed are distinct and named. A finished run is not a passing check; only a check's result says “passed”.
- **Notifications are entry points:** they open the exact session and sheet. Answering from a notification on the lock screen requires unlocking the phone first, and only questions with explicit, safe choices offer buttons there.
- **The computer is always named:** every session, command and file says where it runs.

## What exists and what does not

The `gpui_android` backend provides what these screens assume of the platform: touch, scrolling and flings; the on-screen keyboard with autocorrect and composing; keyboard and system-bar insets; the clipboard with images; file pickers; links; back; notifications with actions; dark mode and high refresh rates.

[Pi for Android](../../crates/pi_android/README.md) builds all twelve screens on it, connected over SSH to Pi Desktop's helper on a computer, with real notifications that open the exact session. Sample sessions that run on their own are one tap away on the first screen. `cargo run -p pi_android --example preview -- <screen>` shows each screen in a phone-sized window.

Still to build: keys held in Android's keystore (the key is a file in the app's private storage for now), QR pairing, and staying connected in the background. The QR shortcut is left out of the connect screen until then.

Not illustrated: disconnected and reconnecting states, several computers at once, tablets and foldables, landscape, and errors from the computer.

## Boundaries

Names, code, commands, times, counts and the assistant's words are sample content. The Android status bar, keyboard and notification shade are drawn plainly as context, not as a specific phone. Mocks use only effects GPUI paints (solid fills, two-stop gradients, shadows, rounded corners, dashed borders).

## Reproduce

```sh
python3 design/android/render.py
```

Needs a local Chromium or headless shell (or set `CHROME`) and Pillow. It renders each `screens/*.html` at 412×915 and 2× scale, then composes [overview.png](overview.png). It uses only local files and the repository's fonts.
