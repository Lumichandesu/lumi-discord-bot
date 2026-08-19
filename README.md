# ✨ Lumi Bot `Ver001`

> Ultra-low latency, featherlight Discord music bot built with **Bun**, **TypeScript**, and a native **FFmpeg** streaming pipeline.

---

### 📊 Benchmark vs. Legacy Stack
Metric              Legacy (Java / Lavalink)    Lumi Bot Ver001 (Native)
────────────────────────────────────────────────────────────────────────
Runtime             Node.js + Java 17 JVM       Bun + TypeScript
Total System RAM    ~580 MB                     ~361 MB (-40% Memory Saved)
Playback Latency    ~3.8s                       < 1.2s (~3x Faster)
Stream Delivery     Java Scraper (Block-prone)  yt-dlp + FFmpeg (Direct PCM)
Supported Sources   YouTube, SoundCloud         YouTube, Spotify, Apple Music, TIDAL, SoundCloud
---

### 🚀 Key Features

* **Bare-Metal Performance:** Built natively on Bun with zero Java runtime overhead.
* **Direct Audio Gateway:** Streams raw 48kHz stereo PCM straight into Discord Voice, avoiding YouTube data-center blocks.
* **Multi-Platform Support:** Resolves tracks seamlessly from **YouTube**, **Spotify**, **Apple Music**, **TIDAL**, and **SoundCloud**.
* **Embedded Micro-Stack:** Integrated SQLite database and ElysiaJS engine for microsecond track logging and zero disk latency.

---

### 🎵 Commands

| Command | Description |
| :--- | :--- |
| `\play <query / URL>` | Stream audio from YouTube, Spotify, Apple Music, TIDAL, or SoundCloud |
| `\pause` / `\resume` | Pause or resume current playback |
| `\skip` / `\stop` | Skip track or stop playback and disconnect |
| `\queue` / `\nowplaying` | Inspect active queue and current track info |
| `\loop` / `\shuffle` | Toggle loop mode (`off` / `track` / `queue`) or randomize playlist |
| `\clear` / `\remove <#>` | Clear playlist or remove a specific track index |
