"""Warm yt-dlp worker for lumi-discord-bot.

The bot starts this script once and keeps it running, so yt-dlp is imported a single time instead
of once per song. Protocol: one JSON object per line on stdin, one per line on stdout.

    argv: <yt-dlp program> <cache dir> <JSON list of yt-dlp command-line options>
    first line out: {"ready": true, "version": "...", "routed": false} or {"ready": false, "error": "..."}
    in:  {"id": 1, "target": "https://..."}      out: {"id": 1, "ok": true, "data": {...}}
                                                 or   {"id": 1, "ok": false, "error": "..."}

The process exits when stdin closes. Requests run on two threads, each with its own YoutubeDL.
"""

import json
import os
import shutil
import sys
import threading
import time
import zipfile
from concurrent.futures import ThreadPoolExecutor

THREADS = 2
# A YoutubeDL keeps caches between requests (faster), but is replaced now and then to bound memory.
REUSE = 25
# An old unpacked copy is deleted once unused for this long (seconds).
KEEP_UNUSED = 24 * 3600
KEEP = ('id', 'title', 'uploader', 'channel', 'duration', 'thumbnail', 'webpage_url', 'url', 'protocol',
        'http_headers', 'filesize', 'is_live', 'live_status')

out = sys.stdout.buffer
# yt-dlp must never write into the protocol stream.
sys.stdout = sys.stderr
out_lock = threading.Lock()


def send(obj):
    line = json.dumps(obj, default=str).encode() + b'\n'
    with out_lock:
        out.write(line)
        out.flush()


def unpacked(zip_path, cache_dir):
    """Extracts the zipapp once, so later starts import cached bytecode instead of the zip."""
    st = os.stat(zip_path)
    name = f'yt-dlp-{st.st_size}-{int(st.st_mtime)}'
    dest = os.path.join(cache_dir, name)
    if not os.path.isfile(os.path.join(dest, 'yt_dlp', '__init__.py')):
        tmp = f'{dest}.tmp{os.getpid()}'
        shutil.rmtree(tmp, ignore_errors=True)
        with zipfile.ZipFile(zip_path) as z:
            z.extractall(tmp)
        try:
            os.rename(tmp, dest)
        except OSError:
            shutil.rmtree(tmp, ignore_errors=True)
            if not os.path.isdir(dest):
                raise
    # Marks the copy as in use; another process sharing the cache dir keeps it then.
    os.utime(dest)
    prune(cache_dir, name)
    return dest


def prune(cache_dir, current):
    """Deletes old copies. A worker of another process (a self-test next to the bot) may still import
    lazily loaded extractors from one, so the newest two and any used in the last day are kept."""
    now = time.time()
    copies = []
    for old in os.listdir(cache_dir):
        path = os.path.join(cache_dir, old)
        if old.startswith('yt-dlp-') and old != current and os.path.isdir(path):
            try:
                copies.append((os.stat(path).st_mtime, path))
            except OSError:
                pass
    copies.sort(reverse=True)
    for mtime, path in copies[1:]:
        if now - mtime > KEEP_UNUSED:
            shutil.rmtree(path, ignore_errors=True)


def load(program, cache_dir):
    """Imports yt-dlp; also returns whether it comes from an unpacked copy instead of `program`."""
    copy = False
    if zipfile.is_zipfile(program):
        try:
            sys.path.insert(0, unpacked(program, cache_dir))
            copy = True
        except Exception as e:
            print(f'could not unpack {program}: {e}; importing from the zip', file=sys.stderr)
            sys.path.insert(0, program)
    import yt_dlp
    return yt_dlp, copy


def parse(yt_dlp, program, args, copy):
    """yt-dlp's options as the yt-dlp command reads them, config files included."""
    if copy:
        # The command reads a portable yt-dlp.conf next to itself, which is the zipapp, not the copy.
        portable = os.path.join(os.path.dirname(os.path.abspath(program)), 'yt-dlp.conf')
        if os.path.isfile(portable):
            args = ['--config-locations', portable, *args]
    real = yt_dlp.parseOpts
    # parse_options(args) alone skips the config files, because explicit arguments are given.
    yt_dlp.parseOpts = lambda argv=None, ignore_config_files='if_override': real(argv, ignore_config_files=False)
    try:
        parsed = yt_dlp.parse_options(args)
    finally:
        yt_dlp.parseOpts = real
    try:
        # The command loads plugins from the directories its options name.
        from yt_dlp.globals import plugin_dirs
        plugin_dirs.value = parsed[1].plugin_dirs
    except (ImportError, AttributeError):
        pass
    return parsed[3]


def slim(info):
    d = {k: info[k] for k in KEEP if info.get(k) is not None}
    entries = info.get('entries')
    if entries is not None:
        d['entries'] = [slim(e) for e in entries if e]
    return d


def error_line(e):
    lines = [line.strip() for line in str(e).splitlines() if line.strip()]
    return lines[-1] if lines else 'yt-dlp failed'


def main():
    program, cache_dir, args = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
    try:
        yt_dlp, copy = load(program, cache_dir)
        opts = parse(yt_dlp, program, args, copy)
    except BaseException as e:  # SystemExit from a bad option too
        send({'ready': False, 'error': error_line(e) or type(e).__name__})
        return
    opts['quiet'] = True
    opts['noprogress'] = True
    local = threading.local()

    def ydl():
        d = getattr(local, 'ydl', None)
        if d is None or local.uses >= REUSE:
            if d is not None and hasattr(d, 'close'):
                d.close()
            d = local.ydl = yt_dlp.YoutubeDL(dict(opts))
            local.uses = 0
        local.uses += 1
        return d

    def handle(req):
        rid = req.get('id')
        try:
            info = ydl().extract_info(req['target'], download=False)
            send({'id': rid, 'ok': True, 'data': slim(info or {})})
        except BaseException as e:
            local.ydl = None
            send({'id': rid, 'ok': False, 'error': error_line(e)})

    def warm():
        # Load the YouTube extractor before the first request needs it.
        ydl().get_info_extractor('Youtube')

    pool = ThreadPoolExecutor(THREADS)
    for _ in range(THREADS):
        pool.submit(warm)
    # Whether requests leave through a proxy, another address or with cookies; the bot's own
    # YouTube search then stays out of the way.
    routed = any(opts.get(k) for k in ('proxy', 'source_address', 'cookiefile', 'cookiesfrombrowser'))
    send({'ready': True, 'version': yt_dlp.version.__version__, 'routed': routed})
    for raw in sys.stdin.buffer:
        try:
            req = json.loads(raw)
        except ValueError:
            continue
        pool.submit(handle, req)
    os._exit(0)


if __name__ == '__main__':
    main()
