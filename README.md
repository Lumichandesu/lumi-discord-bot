# 🎵 Lumi Discord Bot (v1.1)

A minimal, ultra-lightweight, and high-fidelity Discord music bot built with **Bun**, **Discord.js**, and **@discordjs/voice**. Engineered to run 24/7 on low-spec cloud instances (such as Google Cloud `e2-micro` Always Free Tier) with memory consumption under 80 MB.

---

## 🚀 What's New in Version 1.1

- **⚡ Lavalink-Free Native Engine:** Replaced external Java Lavalink nodes with native `@discordjs/voice` and FFmpeg raw PCM streaming, cutting RAM usage from ~500MB+ down to <80MB.
- **🎛️ Interactive Button UI:** Control tracks directly in Discord with interactive buttons (Play/Pause, Skip, Loop, Shuffle, Stop, Queue, Lyrics).
- **🌐 Universal Platform Resolver:** Seamless playback from YouTube, Spotify, Apple Music, and TIDAL via automated metadata bridging.
- **📝 Lyrics & Seek Commands:** Integrated `\lyrics` powered by LRCLIB and precise timestamp hopping via `\seek <mm:ss>`.
- **🚪 Smart Auto-Disconnect:** Automatically leaves empty voice channels after 30 seconds or idle queues after 3 minutes.
- **📊 Real-time System Telemetry:** Upgraded `\ping` command showing WebSocket latency, roundtrip response time, heap RAM, and uptime.
- **⌨️ Command Shortcuts:** Quick aliases added for essential actions (`\p`, `\np`, `\q`, `\s`, `\h`, `\l`).

---

## 📖 Command Reference

Prefix: `\` (Also supports native `/` Slash Commands)

| Command | Alias | Description | Example |
| :--- | :--- | :--- | :--- |
| `\play <query>` | `\p` | Play from YouTube, Spotify, Apple Music, TIDAL, SoundCloud | `\play feel it d4vd` |
| `\pause` | — | Pause current audio playback | `\pause` |
| `\resume` | — | Resume paused playback | `\resume` |
| `\skip` | `\s` | Skip to the next track in queue | `\skip` |
| `\stop` | — | Stop music, clear queue, and disconnect | `\stop` |
| `\seek <time>` | — | Jump to specific timestamp | `\seek 1:45` |
| `\nowplaying` | `\np` | Display detailed metadata of current track | `\nowplaying` |
| `\queue` | `\q` | View upcoming tracks in queue | `\queue` |
| `\loop` | — | Cycle repeat mode (`OFF` ➔ `TRACK` ➔ `QUEUE`) | `\loop` |
| `\shuffle` | — | Randomize queued tracks | `\shuffle` |
| `\remove <#>` | — | Remove a track from queue by position | `\remove 3` |
| `\clear` | — | Clear all upcoming tracks in queue | `\clear` |
| `\lyrics [query]` | `\l` | Search and display track lyrics | `\lyrics` |
| `\ping` | — | Display network ping and memory usage | `\ping` |
| `\help` | `\h` | Display help guide | `\help` |

---

## 🛠️ Setup & Installation

### Prerequisites
- [Bun](https://bun.sh) (v1.1+)
- [FFmpeg](https://ffmpeg.org)
- [yt-dlp](https://github.com/yt-dlp/yt-dlp)

### Installation Steps

1. **Clone the Repository:**
   ```bash
   git clone [https://github.com/Lumichandesu/lumi-discord-bot.git](https://github.com/Lumichandesu/lumi-discord-bot.git)
   cd lumi-discord-bot
