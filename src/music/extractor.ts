import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import type { Provider, ResolvedTrack } from "./types";
import { detectProvider } from "./resolver";

interface YtDlpResult {
  title?: string;
  uploader?: string;
  channel?: string;
  duration?: number;
  thumbnail?: string;
  webpage_url?: string;
  url?: string;
  entries?: YtDlpResult[];
}

interface SpotifyOEmbed {
  title?: string;
  thumbnail_url?: string;
}

function resolveYtDlpBinary(): string {
  if (process.env.YTDLP_PATH && existsSync(process.env.YTDLP_PATH)) {
    return process.env.YTDLP_PATH;
  }
  const homeBin = `${process.env.HOME}/bin/yt-dlp`;
  if (existsSync(homeBin)) {
    return homeBin;
  }
  return "yt-dlp";
}

const YTDLP = resolveYtDlpBinary();

function run(args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(YTDLP, args, {
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });

    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });

    child.once("error", (error) => {
      reject(error);
    });

    child.once("close", (code) => {
      if (code === 0) {
        resolve(stdout.trim());
        return;
      }
      reject(new Error(stderr.trim() || `yt-dlp exited with code ${code}`));
    });
  });
}

function normalizeExtractorError(error: unknown): Error {
  const message = error instanceof Error ? error.message : String(error);

  if (message.includes("Sign in to confirm you're not a bot")) {
    return new Error("YouTube ปฏิเสธการเชื่อมต่อชั่วคราว");
  }
  if (message.includes("HTTP Error 403") || message.includes("HTTP Error 429")) {
    return new Error("แหล่งเพลงจำกัดการเข้าถึงชั่วคราว");
  }
  if (message.includes("Video unavailable") || message.includes("This video is unavailable")) {
    return new Error("เพลงหรือวิดีโอนี้ไม่พร้อมใช้งาน");
  }

  return new Error(message.replace(/^ERROR:\s*/i, "").trim().slice(0, 300));
}

async function extractWithYtDlp(target: string, provider: Provider): Promise<ResolvedTrack> {
  const raw = await run([
    "--no-playlist",
    "-f", "bestaudio/best",
    "--dump-single-json",
    "--skip-download",
    "--no-warnings",
    "--no-progress",
    target,
  ]);

  const parsed = JSON.parse(raw) as YtDlpResult;
  const entry = parsed.entries && parsed.entries.length > 0 ? parsed.entries[0] : parsed;

  if (!entry?.url) {
    throw new Error("ไม่พบ audio stream สำหรับเพลงนี้");
  }

  return {
    provider,
    sourceUrl: entry.webpage_url || target,
    streamUrl: entry.url,
    title: entry.title || "Unknown Title",
    uploader: entry.uploader || entry.channel || "Unknown Artist",
    duration: entry.duration ?? null,
    thumbnail: entry.thumbnail ?? null,
  };
}

async function resolveSpotify(url: string): Promise<ResolvedTrack> {
  const response = await fetch(
    `https://open.spotify.com/oembed?url=${encodeURIComponent(url)}`,
    { signal: AbortSignal.timeout(10_000) }
  );

  if (!response.ok) {
    throw new Error(`Spotify metadata request failed: ${response.status}`);
  }

  const metadata = (await response.json()) as SpotifyOEmbed;
  if (!metadata.title) {
    throw new Error("ไม่สามารถอ่านชื่อเพลงจาก Spotify URL ได้");
  }

  const track = await extractWithYtDlp(`ytsearch1:${metadata.title} official audio`, "spotify");
  track.sourceUrl = url;
  if (metadata.thumbnail_url) {
    track.thumbnail = metadata.thumbnail_url;
  }
  return track;
}

async function resolveAppleMusic(url: string): Promise<ResolvedTrack> {
  const response = await fetch(url, {
    headers: {
      "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    },
    signal: AbortSignal.timeout(10_000),
  });

  if (!response.ok) {
    throw new Error(`Apple Music request failed: ${response.status}`);
  }

  const html = await response.text();
  const ogTitleMatch =
    html.match(/<meta property="og:title" content="([^"]+)"/i) ||
    html.match(/<title>([^<]+)<\/title>/i);

  if (!ogTitleMatch || !ogTitleMatch[1]) {
    throw new Error("ไม่สามารถอ่านชื่อเพลงจาก Apple Music URL ได้");
  }

  const cleanedTitle = ogTitleMatch[1]
    .replace(/&#(\d+);/g, (_, dec) => String.fromCharCode(Number(dec)))
    .replace(/&amp;/g, "&")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/\s+on Apple Music$/i, "")
    .replace(/\s+by\s+/i, " ")
    .trim();

  const ogImageMatch = html.match(/<meta property="og:image" content="([^"]+)"/i);
  const thumbnail = ogImageMatch ? ogImageMatch[1] : null;

  const track = await extractWithYtDlp(`ytsearch1:${cleanedTitle} official audio`, "applemusic");
  track.sourceUrl = url;
  if (thumbnail) {
    track.thumbnail = thumbnail;
  }
  return track;
}

async function resolveTidal(url: string): Promise<ResolvedTrack> {
  const response = await fetch(url, {
    headers: {
      "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    },
    signal: AbortSignal.timeout(10_000),
  });

  if (!response.ok) {
    throw new Error(`Tidal request failed: ${response.status}`);
  }

  const html = await response.text();
  const ogTitleMatch =
    html.match(/<meta property="og:title" content="([^"]+)"/i) ||
    html.match(/<title>([^<]+)<\/title>/i);

  if (!ogTitleMatch || !ogTitleMatch[1]) {
    throw new Error("ไม่สามารถอ่านชื่อเพลงจาก Tidal URL ได้");
  }

  const cleanedTitle = ogTitleMatch[1]
    .replace(/&#(\d+);/g, (_, dec) => String.fromCharCode(Number(dec)))
    .replace(/&amp;/g, "&")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/\s*\|\s*TIDAL$/i, "")
    .replace(/\s+on TIDAL$/i, "")
    .trim();

  const ogImageMatch = html.match(/<meta property="og:image" content="([^"]+)"/i);
  const thumbnail = ogImageMatch ? ogImageMatch[1] : null;

  const track = await extractWithYtDlp(`ytsearch1:${cleanedTitle} official audio`, "tidal");
  track.sourceUrl = url;
  if (thumbnail) {
    track.thumbnail = thumbnail;
  }
  return track;
}

export async function resolveTrack(input: string): Promise<ResolvedTrack> {
  try {
    const trimmed = input.trim();
    const isUrl = /^https?:\/\//i.test(trimmed);

    if (!isUrl) {
      return await extractWithYtDlp(`ytsearch1:${trimmed}`, "search");
    }

    const provider = detectProvider(trimmed);

    if (provider === "spotify") {
      return await resolveSpotify(trimmed);
    }

    if (provider === "applemusic") {
      return await resolveAppleMusic(trimmed);
    }

    if (provider === "tidal") {
      return await resolveTidal(trimmed);
    }

    return await extractWithYtDlp(trimmed, provider || "youtube");
  } catch (error) {
    throw normalizeExtractorError(error);
  }
}
