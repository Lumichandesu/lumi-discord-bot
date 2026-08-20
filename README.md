# 🎵 Lumi Discord Bot (v1.1)

A minimal, ultra-lightweight, and high-fidelity Discord music bot built with **Bun**, **Discord.js**, and **@discordjs/voice**. Engineered to run 24/7 on low-spec clotd instances (e.g., Google Cloud `e2-micro` (Always Free Tier)) with memory consumption under 80 MB.

---

## 🙩 What's New in Version 1.1

- **➡ Lavalink-Free Native Engine:** Replaced external Java Lavalink nodes with native `@discordjs/voice` and FFmpeg raw PCM streaming, cutting RAM usage from ~500MB+ down to <80MB.
- **🎥 Interactive Button UI:** Control tracks directly in Discord with interactive buttons (Play/Pause, Skip, Loop, Shuffle, Stop, Queue, Lyrics).
- **🌐 Universal Platform Resolver:** Playback support for YouTube, Spotify, Apple Music, and TIDAL via automated metadata bridging.
- **�s Lyrics & Seek Commands:** Integrated `\lyrics` powered by LRCLIB and precise timestamp hopping via `\seek <mm:ss>`.
- **🚪 Smart Auto-Disconnect:** Automatically leaves empty voice channels after 30 seconds or idle queues after 3 minutes.
-$**@ Real-time System Telemetry:** Upgraded `\ping` command displaying WebSocket latency, roundtrip time, heap RAM, and uptime.
-$**@ Command Shortcuts:** Added quick aliases (`\p`, `\np`, `\q`, `\s`, `\h`, `\l`).

---

## 🐖 Command Reference

Prefix: `\�` (Also supports native `/`  Slash Commands)

| Command | Alias | Description | Example |
|
--- |
--- |
--- |
--- |
| `\play <query>` | `\p` | Play from YouTube, Spotify, Apple Music, TIDAL, SoundCloud | `\play feel it d4vd` |
| `\pause` | — | Pause current audio playback | `\pause` |
| `\resume` | — | Resume paused playback | `\resume` |
| `\skip` | `\s` | Skip to the next track in queue | `\skip` |
| `\stop` | — | Stop music, clear queue, and disconnect | `\stop` |
| `\seek <time>` | — | Jump to specific timestamp | `\seek 1:45` |
| `\nowplaying` | `\np` | Display detailed metadata of current track | `\nowplaying` |
| `\queue` | `\qp` | View upcoming tracks in queue | `\queue` |
| `\loop` | — | Cycle repeat mode (`OFF` ➠ `TRACK` ➠ `QUEUE`) | `\loop` |
| `\shuffle` | — | Randomize queued tracks | `\shuffle` |
| `\remove <#>` | — | Remove a track from queue by position | `\remove 3` |
| `\clear` | — | Clear all upcoming tracks in queue | `\clear` |
| `\lyrics [query]` | `\l` | Search and display track lyrics | `\lyrics` |
| `\ping` | — | Display network ping and memory usage | `\ping` |
| `\help` | `\h` | Display help guide | `\help` |

---

3! 💌 Setup & Installation

### Prerequisites
- Bun (v1.1+)
- FFmpeg
- yt-dlp

### Installation Steps
1. Clone the repository: `git clone https://github.com/Lumichandesu/lumi-discord-bot.git`
2. Install dependencies: `bun install`
3. Configure environment: `cp .env.example .env`
4. Start the bot: `bun run src/index.ts`

---

## �� License
MIT License © 2026 Lumichandesu
