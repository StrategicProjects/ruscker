# Minimal interactive app to e2e-test Ruscker's URL rewriting +
# WebSocket proxying. Pure aiohttp: HTTP + WS in one process.
import asyncio
from aiohttp import web, WSMsgType

INDEX = """<!doctype html>
<html>
<head>
  <title>wsapp</title>
  <link rel="stylesheet" href="/static/app.css">
  <script src="/static/app.js"></script>
</head>
<body>
  <h1>wsapp</h1>
  <div id="out">init</div>
</body>
</html>"""

APP_JS = """// served from /static/app.js
console.log("app.js loaded");
fetch('/api/ping').then(r => r.json()).then(j => {
  document.getElementById('out').textContent = 'fetch:' + j.pong;
});
var ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws');
ws.onopen = function(){ ws.send('hello'); };
ws.onmessage = function(e){ document.getElementById('out').textContent += ' ws:' + e.data; };
"""

async def index(request):
    return web.Response(text=INDEX, content_type='text/html')

async def app_js(request):
    return web.Response(text=APP_JS, content_type='application/javascript')

async def app_css(request):
    return web.Response(text="body{font-family:sans-serif}", content_type='text/css')

async def ping(request):
    return web.json_response({"pong": "ok"})

async def ws_handler(request):
    ws = web.WebSocketResponse()
    await ws.prepare(request)
    async for msg in ws:
        if msg.type == WSMsgType.TEXT:
            await ws.send_str("echo:" + msg.data)
            if msg.data == "close":
                await ws.close()
    return ws

async def heartbeat():
    # Emit a flushed line every 2s so the live-logs follow path
    # has a steady stream to forward. stdout is unbuffered via
    # PYTHONUNBUFFERED in the Dockerfile, so each line reaches
    # `docker logs` immediately.
    n = 0
    while True:
        print(f"heartbeat {n}", flush=True)
        n += 1
        await asyncio.sleep(2)

async def on_startup(app):
    app["hb"] = asyncio.create_task(heartbeat())

async def on_cleanup(app):
    app["hb"].cancel()

app = web.Application()
app.add_routes([
    web.get('/', index),
    web.get('/static/app.js', app_js),
    web.get('/static/app.css', app_css),
    web.get('/api/ping', ping),
    web.get('/ws', ws_handler),
])
app.on_startup.append(on_startup)
app.on_cleanup.append(on_cleanup)

if __name__ == '__main__':
    web.run_app(app, host='0.0.0.0', port=8080)
