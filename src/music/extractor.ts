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

interface QuickSearchMetadata {
  videoId: string;
  title: string;
  uploader: string;
  duration: number | null;
  thumbnail: string | null;
}

interface CachedItem {
  track: ResolvedTrack;
  expiresAt: number;
}
const searchCache = new Map<string, CachedItem>();

function getFromCache(key: string): ResolvedTrack | null {
  const cached = searchCache.get(key.toLowerCase().trim());
  if (!cached) return null;
  if (Date.now() > cached.expiresAt) {
    searchCache.delete(key.toLowerCase().trim());
    return null;
  }
  return cached.track;
}

function setToCache(key: string, track: ResolvedTrack) {
  if (searchCache.size > 150) {
    const oldestKey = searchCache.keys().next().value;
    if (oldestKey) searchCache.delete(oldestKey);
  }
  searchCache.set(key.toLowerCase().trim(), {
    track,
    expiresAt: Date.now() + 60 * 60 * 1000,
  });
}

class BackgroundQueue {
  private queue: (() => Promise<void>)[] = [];
  private isProcessing = false;

  async add<T>(task: () => Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      this.queue.push(async () => {
        try {
          const res = await task();
          resolve(res);
        } catch (err) {
          reject(err);
        }
      });
      this.tick();
    });
  }

  private async tick() {
    if (this.isProcessing || this.queue.length === 0) return;
    this.isProcessing = true;
    const current = this.queue.shift();
    if (current) {
      try {
        await current();
      } finally {
        this.isProcessing = false;
        this.tick();
      }
    }
  }
}

const backgroundQueue = new BackgroundQueue();

function resolveYtDlpBinary(): string {
  if (process.env.YTDLP_PATH && existsSync(process.env.YTDLP_PATH)) {
    return process.env.YTDLP_PATH;
  }
  const homeBin = `${process.env.HOME}/bin/yt-dlp`;
  if (existsSync(homeBin)) return homeBin;
  return "yt-dlp";
}

const YTDLP = resolveYtDlpBinary();

function runBackgroundProcess(args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn("nice", ["-n", "19", YTDLP, ...args], {
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk) => { stdout += chunk.toString(); });
    child.stderr.on("data", (chunk) => { stderr += chunk.toString(); });
    child.once("error", (err) => { reject(err); });

    child.once("close", (code) => {
      if (code === 0) {
        resolve(stdout.trim());
      } else {
        reject(new Error(stderr.trim() || `Process exited with code ${code}`));
      }
    });
  });
}

// 1. Apple Music Metadata Resolver
async function fetchAppleMusicMetadata(url: string): Promise<string | null> {
  try {
    const idMatch = url.match(/[?&]i=(\d+)/) || url.match(/\/(\d+)(?:\?|$)/);
    if (idMatch && idMatch[1]) {
      const itunesRes = await fetch(`https://itunes.apple.com/lookup?id=${idMatch[1]}`, {
        signal: AbortSignal.timeout(3000),
      });
      if (itunesRes.ok) {
        const data: any = await itunesRes.json();
        if (data?.results?.length > 0) {
          const item = data.results[0];
          const title = item.trackName || item.collectionName || "";
          const artist = item.artistName || "";
          if (title) return `${title} ${artist}`.trim();
        }
      }
    }

    const cleanUrl: string = url.split("?")[0] || url;
    const res = await fetch(cleanUrl, {
      headers: {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Accept-Language": "en-US,en;q=0.9",
      },
      signal: AbortSignal.timeout(3000),
    });
    if (res.ok) {
      const html = await res.text();
      const ogTitle = html.match(/<meta\s+property=["']og:title["']\s+content=["']([^"']+)["']/i)?.[1]
        || html.match(/<meta\s+content=["']([^"']+)["']\s+property=["']og:title["']/i)?.[1];
      const ogDesc = html.match(/<meta\s+property=["']og:description["']\s+content=["']([^"']+)["']/i)?.[1]
        || html.match(/<meta\s+content=["']([^"']+)["']\s+property=["']og:description["']/i)?.[1];

      if (ogTitle) {
        const title = ogTitle.trim();
        let artist = "";
        if (ogDesc) {
          const parts = ogDesc.split(/[·•]/);
          const firstPart = parts[0];
          if (firstPart) artist = firstPart.trim();
        }
        return artist ? `${title} ${artist}` : title;
      }
    }

    const slugMatch = url.match(/(?:song|album)\/([^\/]+)\/\d+/i);
    if (slugMatch && slugMatch[1]) {
      return slugMatch[1].replace(/-/g, " ").trim();
    }
  } catch (err) {
    console.error("[AppleMusic Resolver Error]", err);
  }
  return null;
}

// 2. Spotify Metadata Resolver
async function fetchSpotifyMetadata(url: string): Promise<string | null> {
  try {
    const cleanUrl: string = url.split("?")[0] || url;
    const oembedUrl = `https://open.spotify.com/oembed?url=${encodeURIComponent(cleanUrl)}`;
    const res = await fetch(oembedUrl, { signal: AbortSignal.timeout(3500) });
    if (res.ok) {
      const data: any = await res.json();
      const title = data?.title || "";
      const artist = data?.author_name || "";
      if (title) return `${title} ${artist}`.trim();
    }
  } catch (err) {
    console.error("[Spotify Resolver Error]", err);
  }
  return null;
}

// 3. TIDAL Metadata Resolver
async function fetchTidalMetadata(url: string): Promise<string | null> {
  try {
    const cleanUrl: string = url.split("?")[0]?.replace(/\/u\/?$/, "") || url;
    const res = await fetch(cleanUrl, {
      headers: {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Accept-Language": "en-US,en;q=0.9",
      },
      signal: AbortSignal.timeout(3500),
    });

    if (res.ok) {
      const html = await res.text();
      const ogTitle = html.match(/<meta\s+property=["']og:title["']\s+content=["']([^"']+)["']/i)?.[1]
        || html.match(/<meta\s+content=["']([^"']+)["']\s+property=["']og:title["']/i)?.[1];
      const ogDesc = html.match(/<meta\s+property=["']og:description["']\s+content=["']([^"']+)["']/i)?.[1]
        || html.match(/<meta\s+content=["']([^"']+)["']\s+property=["']og:description["']/i)?.[1];

      if (ogTitle) {
        const title = ogTitle.replace(/\s*\|\s*TIDAL.*$/i, "").replace(/\s*-\s*TIDAL.*$/i, "").trim();
        let artist = "";
        if (ogDesc) {
          const match = ogDesc.match(/(?:by|from)\s+([^,.]+?)(?:\s+on\s+TIDAL|\.|$)/i);
          if (match && match[1]) {
            artist = match[1].trim();
          }
        }
        return artist ? `${title} ${artist}` : title;
      }

      const titleTag = html.match(/<title>([^<]+)<\/title>/i)?.[1];
      if (titleTag) {
        return titleTag.replace(/\|\s*TIDAL/i, "").replace(/-\s*TIDAL/i, "").trim();
      }
    }
  } catch (err) {
    console.error("[Tidal Resolver Error]", err);
  }
  return null;
}

// 4. YouTube Fast Search
async function fastYouTubeSearch(query: string): Promise<QuickSearchMetadata | null> {
  try {
    const res = await fetch("https://www.youtube.com/youtubei/v1/search", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        context: {
          client: {
            clientName: "WEB",
            clientVersion: "2.20240101.01.00",
            hl: "en",
            gl: "US",
          },
        },
        query,
      }),
      signal: AbortSignal.timeout(3500),
    });

    if (!res.ok) return null;
    const data: any = await res.json();
    const sections = data?.contents?.twoColumnSearchResultsRenderer?.primaryContents?.sectionListRenderer?.contents || [];

    for (const section of sections) {
      const items = section?.itemSectionRenderer?.contents || [];
      for (const item of items) {
        const video = item?.videoRenderer;
        if (video?.videoId) {
          const title = video?.title?.runs?.[0]?.text || "Unknown Title";
          const uploader = video?.ownerText?.runs?.[0]?.text || "Unknown Artist";
          const lengthStr = video?.lengthText?.simpleText || "";
          let duration: number | null = null;
          if (lengthStr) {
            const parts: number[] = lengthStr.split(":").map(Number);
            duration = parts.reduce((acc: number, curr: number) => acc * 60 + curr, 0);
          }
          const thumbnails = video?.thumbnail?.thumbnails || [];
          const thumbnail = thumbnails.length > 0 ? thumbnails[thumbnails.length - 1]?.url ?? null : null;

          return {
            videoId: video.videoId,
            title,
            uploader,
            duration,
            thumbnail,
          };
        }
      }
    }
  } catch {
    return null;
  }
  return null;
}

// 5. Stream Extraction
async function extractDirectStreamUrl(targetUrl: string): Promise<string> {
  const streamUrl = await runBackgroundProcess([
    "-g",
    "-f", "251/ba/b",
    "--no-playlist",
    "--no-warnings",
    "--no-progress",
    "--extractor-args", "youtube:player_client=android",
    "--socket-timeout", "5",
    targetUrl,
  ]);

  const lines = streamUrl.split("\n").map((l) => l.trim()).filter(Boolean);
  const firstLine = lines[0];
  if (!firstLine || !firstLine.startsWith("http")) {
    throw new Error("No playable audio stream returned from source.");
  }
  return firstLine;
}

async function extractWithYtDlp(target: string, provider: Provider): Promise<ResolvedTrack> {
  const raw = await runBackgroundProcess([
    "--no-playlist",
    "-f", "251/ba/b",
    "--dump-single-json",
    "--skip-download",
    "--no-warnings",
    "--no-progress",
    "--extractor-args", "youtube:player_client=android",
    "--socket-timeout", "5",
    target,
  ]);

  const parsed = JSON.parse(raw) as YtDlpResult;
  const entry = parsed.entries && parsed.entries.length > 0 ? parsed.entries[0] : parsed;

  if (!entry?.url) {
    throw new Error("No suitable audio stream found.");
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

export async function resolveTrack(input: string): Promise<ResolvedTrack> {
  const trimmed = input.trim();
  
  const cached = getFromCache(trimmed);
  if (cached) return cached;

  return backgroundQueue.add(async () => {
    const doubleCheck = getFromCache(trimmed);
    if (doubleCheck) return doubleCheck;

    let searchQuery = trimmed;
    let isBridgeLink = false;

    if (/music\.apple\.com|itunes\.apple\.com/i.test(trimmed)) {
      isBridgeLink = true;
      const meta = await fetchAppleMusicMetadata(trimmed);
      if (meta) {
        searchQuery = meta;
      } else {
        const slug = trimmed.split("/").filter(Boolean).slice(-2, -1)[0]?.replace(/-/g, " ");
        searchQuery = slug || trimmed;
      }
    } else if (/open\.spotify\.com/i.test(trimmed)) {
      isBridgeLink = true;
      const meta = await fetchSpotifyMetadata(trimmed);
      if (meta) {
        searchQuery = meta;
      }
    } else if (/tidal\.com/i.test(trimmed)) {
      isBridgeLink = true;
      const meta = await fetchTidalMetadata(trimmed);
      if (meta) {
        searchQuery = meta;
      }
    }

    const isUrl = /^https?:\/\//i.test(searchQuery);
    let result: ResolvedTrack;

    if (!isUrl || isBridgeLink) {
      const meta = await fastYouTubeSearch(searchQuery);
      if (meta) {
        const ytUrl = `https://www.youtube.com/watch?v=${meta.videoId}`;
        const streamUrl = await extractDirectStreamUrl(ytUrl);
        result = {
          provider: "youtube",
          sourceUrl: ytUrl,
          streamUrl,
          title: meta.title,
          uploader: meta.uploader,
          duration: meta.duration,
          thumbnail: meta.thumbnail,
        };
      } else {
        result = await extractWithYtDlp(`ytsearch1:${searchQuery}`, "search");
      }
    } else {
      const provider = detectProvider(searchQuery);
      result = await extractWithYtDlp(searchQuery, provider || "youtube");
    }

    setToCache(trimmed, result);
    return result;
  });
}
