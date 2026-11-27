# Lumi Discord Bot: a music bot for Discord built with Bun and discord.js

Lumi is a self-hosted Discord music bot written in TypeScript for people who want to run their own bot for a server. It plays audio into voice channels from YouTube and SoundCloud links or search text, and it resolves Spotify, Apple Music and TIDAL links by looking up the track title and finding a match on YouTube. It runs on Bun with discord.js, `@discordjs/voice`, FFmpeg and yt-dlp.

## Features

- Commands use the `\` prefix. A smaller set also exists as slash commands.
- Playback buttons on the now-playing message: pause, skip, loop, shuffle, stop, queue and lyrics.
- Queue controls: skip, loop, shuffle, remove a track, clear, and seek to a time.
- Lyrics are fetched from LRCLIB.
- Audio is streamed through FFmpeg straight into `@discordjs/voice`, with no separate Lavalink server.
- The bot leaves a voice channel about 30 seconds after the last person leaves, and stops after 3 minutes of idle playback.

## Commands

| Command | Alias | What it does |
| :--- | :--- | :--- |
| `\play <query or URL>` | `\p` | Play or queue a track |
| `\pause`, `\resume` | | Pause or resume |
| `\skip` | `\s` | Skip the current track |
| `\stop` | | Stop, clear the queue and disconnect |
| `\seek <time>` | | Jump to a time such as `1:45` |
| `\nowplaying` | `\np` | Show the current track |
| `\queue` | `\q` | Show the queue |
| `\loop` | | Cycle the repeat mode |
| `\shuffle` | | Shuffle the queue |
| `\remove <position>` | | Remove a queued track |
| `\clear` | | Clear upcoming tracks |
| `\lyrics [query]` | `\l` | Show lyrics |
| `\ping` | | Show latency, memory use and uptime |
| `\help` | `\h` | List commands |

Slash commands registered by `src/deploy-commands.ts`: `play`, `skip`, `stop`, `nowplaying`, `queue`, `pause`, `resume`, `shuffle`, `clear`.

## Setup

Requirements: [Bun](https://bun.sh), [FFmpeg](https://ffmpeg.org) and [yt-dlp](https://github.com/yt-dlp/yt-dlp) available on the machine, and a bot application created in the Discord Developer Portal. The bot needs the Message Content intent enabled there, because it reads prefix commands.

```bash
git clone https://github.com/Lumichandesu/lumi-discord-bot.git
cd lumi-discord-bot
bun install
cp .env.example .env
```

Fill in `.env`. `DISCORD_TOKEN` and `DISCORD_CLIENT_ID` are required; `DISCORD_GUILD_ID` is optional. Never commit `.env`.

```bash
bun run src/deploy-commands.ts   # register slash commands
bun run start                    # start the bot
```

Other scripts: `bun run dev` restarts on file changes, and `bun run typecheck` runs the TypeScript compiler.

## Notes

- Spotify, Apple Music and TIDAL links are only used to read the track name; the audio itself comes from YouTube through yt-dlp, so results can differ from the original track.
- yt-dlp and the services it reads change often, so keep it updated if playback stops working.
- There are no automated tests.

## License

MIT, see `LICENSE`.
