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

## 🚀 Quick Start & Self-Hosting

### 1. Prerequisites
* [Bun](https://bun.sh) (v1.1+)
* `ffmpeg` installed on your system (`sudo apt install ffmpeg`)
* `yt-dlp` installed in `$PATH` or `~/bin/yt-dlp`

### 2. Installation
```bash
git clone [https://github.com/Lumichandesu/lumi-discord-bot.git](https://github.com/Lumichandesu/lumi-discord-bot.git)
cd lumi-discord-bot
bun install
