<div align="center">

# Courier (syncourier)

**Reach your computer through messengers: notifications and conversations with Claude Code from WhatsApp and Telegram — text, voice, files.**

[Русский](README.ru.md)

</div>

Courier is a synshell service. It links to WhatsApp as a linked device (like WhatsApp Web) and listens only to your
**“Message yourself”** chat. Whatever you write there goes to Claude Code — Courier's own session `courier`; its
replies, system notifications and events from your other Claude sessions come back to the same chat. **Telegram** works
through your own bot: create it with @BotFather (`/newbot`), paste the token in Settings → Courier, scan the QR code and
press Start — the bot then answers only you. With both linked, replies go to the messenger you wrote from last. E-mail is next.

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

## Telegram remote panel

The bot's menu button **"Пульт"** opens a Mini App (syngui → WebAssembly, crate `syn-courier-panel`) instead of
commands: Claude "courier" and channel state, New conversation / Stop / Screenshot buttons, live Claude sessions
(tap one to address it, like `/to`), recent messages and an input field. The service serves the page and its API on
`127.0.0.1:51871` and exposes it with `cloudflared tunnel --url` (quick tunnel, no Cloudflare account), setting the
new `https://….trycloudflare.com` address as the bot's menu button on every start. Every API call carries the
Telegram-signed `initData`, checked against the bot token, freshness and the bot owner's user id.

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
