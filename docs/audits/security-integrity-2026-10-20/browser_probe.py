import http.server, threading, tempfile, subprocess, pathlib, json
requests=[]
html=pathlib.Path('/tmp/vak-security-audit-2026-10-20/rendered.html').read_text()
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        requests.append(self.path)
        self.send_response(200)
        if self.path=='/':
            self.send_header('Content-Type','text/html'); self.end_headers()
            self.wfile.write(('<!doctype html><body>'+html.replace('PORT',str(self.server.server_port))+'</body>').encode())
        else:
            self.send_header('Content-Type','image/png'); self.end_headers()
    def log_message(self,*a): pass
s=http.server.ThreadingHTTPServer(('127.0.0.1',0),H)
threading.Thread(target=s.serve_forever,daemon=True).start()
with tempfile.TemporaryDirectory(prefix='vak-audit-browser-') as profile:
    p=subprocess.Popen(['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome','--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--user-data-dir='+profile,'--dump-dom','--virtual-time-budget=1500','http://127.0.0.1:'+str(s.server_port)+'/'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try: p.wait(timeout=15)
    except subprocess.TimeoutExpired:
        p.kill(); p.wait()
s.shutdown()
assert any('/collect?marker=SYNTHETIC_PRIVATE_123'==r for r in requests), requests
print(json.dumps({'confirmed':'real markdown renderer triggers an automatic image request carrying synthetic private marker','requests':requests}))
