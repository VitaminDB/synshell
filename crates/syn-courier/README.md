<div align="center">

# Courier (syncourier)

**Reach your computer through messengers: notifications and conversations with Claude Code from WhatsApp — text, voice, files.**

[Русский](README.ru.md)

</div>

Courier is a synshell service. It links to WhatsApp as a linked device (like WhatsApp Web) and listens only to your
**“Message yourself”** chat. Whatever you write there goes to Claude Code — Courier's own session `courier`; its
replies, system notifications and events from your other Claude sessions come back to the same chat. Telegram and
e-mail are the next channels.

## Linking

1. **Courier** icon on the top panel → **Link** (also **Settings → Courier**, or `syncourier link` in a terminal).
2. On the phone: WhatsApp → **Settings → Linked devices → Link a device**, scan the QR code.
3. Send `/help` to “Message yourself”.

## Talking

Just write — Claude Code answers and works on this machine like your terminal sessions.

| Command | |
|---|---|
| `/to NAME text` | pass a message to a live Claude session; its reply comes back here |
| `/sessions` | live Claude sessions |
| `/new [DIR]` | new conversation |
| `/stop` | stop the current answer |
| `/status` | channels, Claude, notification mode |
| `/notify claude\|away\|all` | which system notifications to forward |

Voice messages are transcribed by the synaptix service; Claude sometimes answers with voice (synaptix TTS). Photos,
videos, documents and audio are saved to `~/Downloads/Курьер` and handed to Claude with your caption; Claude can send
files back (`syncourier send`).

## Notifications

Settings → Courier: system notifications (*off* · *when I'm away* · *all*), Claude Code events (*when I'm away* ·
*always* · *never*), idle minutes, excluded apps. “Away” = screen locked or N minutes without input.

## Command line

```text
syncourier status | link | unlink | reload
syncourier notify TEXT [--title T] [--source claude]
syncourier send FILE [CAPTION] [--voice]
syncourier say TEXT
syncourier ask TEXT
syncourier hook          (Claude Code hooks: Stop, Notification)
```

Settings live in `~/.config/synshell/courier.toml`; the WhatsApp session in `~/.local/share/syncourier/` (owner only).
Courier reacts only to your own “Message yourself” chat. Anyone with access to your WhatsApp can control the computer
through Claude — keep the phone locked. The WhatsApp library (whatsapp-rust) is unofficial.
