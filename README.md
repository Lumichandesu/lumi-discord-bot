# Lumi Discord Bot: a self-hosted Discord music bot in Rust

Lumi is a self-hosted Discord music bot written in Rust with serenity and songbird. It plays audio in voice channels from YouTube and SoundCloud links or plain search text, using yt-dlp to find the audio stream. Spotify, Apple Music and TIDAL links are read only for the track title and artist; the audio then comes from a YouTube search for that text. It runs as a single binary with no Lavalink server and no FFmpeg.


## Features

- 15 commands, each available both as a `\` prefix command and as a slash command.
- A now-playing message with buttons: pause or resume, skip, loop, shuffle, stop, queue and lyrics.
- Queue controls: skip, loop (off, track, queue), shuffle, remove a track, clear, and seek to a time.
- Lyrics from LRCLIB.
- YouTube Opus audio is passed to Discord without decoding and re-encoding when the format allows it.
- The bot leaves the voice channel 30 seconds after the last person leaves, and 3 minutes after the queue runs out.

## Commands

| Command | Alias | What it does |
| :--- | :--- | :--- |
| `\play <query or URL>` | `\p` | Play or queue a track |
| `\pause`, `\resume` | | Pause or resume |
| `\skip` | `\s` | Skip the current track |
| `\stop` | | Stop, clear the queue and leave the voice channel |
| `\seek <time>` | | Jump to a time such as `1:45`, `1:02:03` or `90` |
| `\nowplaying` | `\np` | Show the current track |
| `\queue` | `\q` | Show the queue |
| `\loop` | | Cycle the repeat mode |
| `\shuffle` | | Shuffle the queue |
| `\remove <position>` | | Remove a queued track |
| `\clear` | | Clear upcoming tracks |
| `\lyrics [query]` | `\l` | Show lyrics for the current track or a search |
| `\ping` | | Show latency, memory use and uptime |
| `\help` | `\h` | List commands |

The slash commands have the same names, for example `/play query:...` and `/seek time:1:45`.

## Setup

1. Install Rust (https://rustup.rs) and yt-dlp (https://github.com/yt-dlp/yt-dlp). FFmpeg is not needed.
2. Create a bot application in the Discord Developer Portal and enable the Message Content intent, which the prefix commands need.
3. Copy `.env.example` to `.env` and fill it in. Never commit `.env`.

   | Variable | Required | Meaning |
   | :--- | :--- | :--- |
   | `DISCORD_TOKEN` | yes | Bot token |
   | `DISCORD_CLIENT_ID` | for `--deploy` | Application ID |
   | `DISCORD_GUILD_ID` | no | With `--deploy`, register commands in this server only (updates instantly) instead of globally |
   | `YTDLP_PATH` | no | Path to the yt-dlp program; otherwise `~/bin/yt-dlp`, then `yt-dlp` on `PATH` |
   | `RUST_LOG` | no | Log filter, default `lumi_discord_bot=info,serenity=warn,songbird=warn` |

4. Build and run:

   ```bash
   cargo build --release
   ./target/release/lumi-discord-bot --deploy     # register slash commands once
   ./target/release/lumi-discord-bot              # start the bot
   ```

`./target/release/lumi-discord-bot --selftest` checks the parts that do not need Discord: it searches YouTube through yt-dlp, opens the audio stream and checks that it can be decoded. It skips the test if yt-dlp is not installed.

## Development

```bash
cargo test
cargo clippy -- -D warnings
```

Unit tests cover the queue, shuffle, loop modes, seek time parsing, link classification and the parsing of yt-dlp and web page metadata.

## Notes

- Audio for Spotify, Apple Music and TIDAL links comes from YouTube, so the result can differ from the original track.
- yt-dlp and the sites it reads change often. Update yt-dlp if playback stops working.
- yt-dlp runs one request at a time and results are cached for an hour (up to 150 entries), to keep CPU use low on small servers. On Linux and macOS it runs with `nice -n 19`.

## License

MIT, see `LICENSE`.
