"""Warm yt-dlp worker for lumi-discord-bot.

The bot starts this script once and keeps it running, so yt-dlp is imported a single time instead
of once per song. Protocol: one JSON object per line on stdin, one per line on stdout.

    argv: <yt-dlp program> <cache dir> <JSON list of yt-dlp command-line options>
    first line out: {"ready": true, "version": "..."} or {"ready": false, "error": "..."}
    in:  {"id": 1, "target": "https://..."}      out: {"id": 1, "ok": true, "data": {...}}
                                                 or   {"id": 1, "ok": false, "error": "..."}

The process exits when stdin closes. Requests run on two threads, each with its own YoutubeDL.
"""

import json
import os
import shutil
import sys
import threading
import zipfile
from concurrent.futures import ThreadPoolExecutor

THREADS = 2
# A YoutubeDL keeps caches between requests (faster), but is replaced now and then to bound memory.
REUSE = 25
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
    for old in os.listdir(cache_dir):
        if old.startswith('yt-dlp-') and old != name:
            shutil.rmtree(os.path.join(cache_dir, old), ignore_errors=True)
    return dest


def load(program, cache_dir):
    if zipfile.is_zipfile(program):
        try:
            sys.path.insert(0, unpacked(program, cache_dir))
        except Exception as e:
            print(f'could not unpack {program}: {e}; importing from the zip', file=sys.stderr)
            sys.path.insert(0, program)
    import yt_dlp
    return yt_dlp


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
        yt_dlp = load(program, cache_dir)
        opts = yt_dlp.parse_options(args)[3]
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
    send({'ready': True, 'version': yt_dlp.version.__version__})
    for raw in sys.stdin.buffer:
        try:
            req = json.loads(raw)
        except ValueError:
            continue
        pool.submit(handle, req)
    os._exit(0)


if __name__ == '__main__':
    main()
