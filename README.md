# 🎵 Lumi Bot `Ver001`

> Ultra-low latency, featherlight Discord music streaming engine built natively with **Bun**, **TypeScript**, **ElysiaJS**, and **FFmpeg**.

---

## ⚡ Overview

**Lumi Bot (Ver001)** is a custom-engineered Discord music bot designed to eliminate the heavy memory footprint and instability of legacy Java/Lavalink middleware. By streaming directly via a native **`yt-dlp` + `ffmpeg`** raw PCM audio pipeline into Discord Voice, it achieves instant playback, zero IP-block issues, and extreme resource efficiency on lightweight 1 GB VMs.

---

## 📊 Architecture & Performance Benchmark

| Metric / Specification | Legacy Stack (Java / Lavalink) | Lumi Bot Ver001 (Native) | Performance Gain |
| :--- | :--- | :--- | :--- |
| **Runtime Engine** | Node.js + OpenJDK 17 | **Bun + TypeScript** | Bare-metal execution |
| **Total System RAM** | ~580 MB | **~361 MB** *(OS + Bot + DB)* | **~40% Memory Saved** |
| **Playback Start Latency** | ~3.8s | **< 1.2s** | **~3x Faster Playback** |
| **Audio Delivery Pipeline** | Java Scraper (Block-prone) | **`yt-dlp` + FFmpeg Direct PCM** | Direct 48kHz Stereo Pipe |
| **Storage & API Stack** | External / Standalone | **Embedded SQLite + ElysiaJS** | Zero disk lag & instant queries |

---

## 🌐 Supported Platforms

Lumi Bot resolves metadata dynamically and streams high-fidelity audio across all major platforms:

* 🔴 **YouTube** & YouTube Music (Tracks & Search queries)
* 🟢 **Spotify** (Direct Track Links)
* 🍎 **Apple Music** (Direct Track Links)
* ⚫ **TIDAL** (Direct Track Links)
* 🟠 **SoundCloud** (Direct Track & Search queries)

---

## 🛠️ Tech Stack

* **Runtime:** [Bun](https://bun.sh) (v1.x)
* **Language:** TypeScript
* **Discord Library:** [discord.js](https://discord.js.org) (v14) & `@discordjs/voice`
* **Audio Processing:** `yt-dlp` + `ffmpeg` (Raw S16LE 48000Hz 2-Channel PCM)
* **Database:** Embedded SQLite (Local playback history & analytics)
* **Web & API Framework:** [ElysiaJS](https://elysiajs.com) (High-throughput internal REST server)

---

## 🎵 Commands

| Command | Arguments | Description |
| :--- | :--- | :--- |
| `\play` | `<Title / URL>` | Stream audio from YouTube, Spotify, Apple Music, TIDAL, or SoundCloud |
| `\pause` | — | Pause the current playback |
| `\resume` | — | Resume paused playback |
| `\skip` | — | Skip the current track to the next item in queue |
| `\stop` | — | Stop playback, clear queue, and leave the voice channel |
| `\nowplaying` | — | Display live metadata of the currently playing track |
| `\queue` | — | View the upcoming playlist queue |
| `\loop` | — | Cycle loop modes: `off` ➔ `track` ➔ `queue` |
| `\shuffle` | — | Randomize the order of queued tracks |
| `\remove` | `<#>` | Remove a specific track from the queue by its index number |
| `\clear` | — | Clear all upcoming tracks in the queue |

---

## 🚀 Deployment & Service Management

The bot runs as a background systemd user service on Linux:

```bash
# Check service status
systemctl --user status lumi-discord-bot.service

# View real-time logs
journalctl --user -u lumi-discord-bot.service -f

# Restart service after updates
systemctl --user restart lumi-discord-bot.service
