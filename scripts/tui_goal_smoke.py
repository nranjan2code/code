
"""Live TUI goal-mode smoke: /goal arm -> prompt -> audited completion."""
import os, pty, select, subprocess, sys, time, fcntl, struct, termios
BIN=sys.argv[1]
ENV={**os.environ,
 "VAK_HOME":"/tmp/vak-live/home",
 "TERM":"xterm-256color"}
os.makedirs("/tmp/vak-live/goalproj/.vak",exist_ok=True)
open("/tmp/vak-live/goalproj/.vak/config.toml","w").write('permission_mode = "full-access"\n')
master,slave=pty.openpty()
fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",50,120,0,0))
p=subprocess.Popen([BIN,"tui","--trust"],stdin=slave,stdout=slave,stderr=open("/tmp/tui_stderr.txt","ab",0),env=ENV,cwd="/tmp/vak-live/goalproj",close_fds=True)
os.close(slave)
out=b""
def pump(sec):
    global out
    end=time.time()+sec
    while time.time()<end:
        r,_,_=select.select([master],[],[],0.1)
        if master in r:
            try: c=os.read(master,65536)
            except OSError: return False
            if not c: return False
            out+=c
    return True
def send(t): os.write(master,t.encode())
try:
    pump(1.5)
    send("/goal\r"); pump(1.0); send("\x1b"); pump(0.5)
    # two-phase: let the palette filter first, then an explicit Enter
    send("/goal Create goal-live.txt with content ok-7 -- verify: test -f goal-live.txt && grep -qx ok-7 goal-live.txt; final message states creation")
    pump(0.8)
    send("\r")
    pump(1.5)
    send("Go.\r")
    # wait for audited run to complete
    end=time.time()+150
    while time.time()<end:
        pump(2)
        if b"completed" in out: break
    send("\x03"); pump(1.0); send("y"); pump(1.5)
    send("/exit\r"); pump(1.5)
    send("\x03"); pump(0.8)
except Exception as e:
    print("driver err:",e)
try: p.terminate()
except Exception: pass
deadline=time.time()+10
while time.time()<deadline:
    r,_,_=select.select([master],[],[],0.2)
    if master in r:
        try:
            c=os.read(master,65536)
            if not c: break
            out+=c
        except OSError: break
    elif p.poll() is not None: break
code=p.wait(timeout=5)
import re
t=re.sub(rb"\x1b\[[0-9;]*[a-zA-Z]",b"",out).decode("utf-8","replace")
checks={
 "arm feedback": "🎯 goal armed" in t,
 "status probe renders": ("no goal armed" in t) or ("next prompt runs audited" in t),
 "usage hint": "bare = status" in t or "no goal armed" in t,
 "audit continuation visible": "[goal-audit]" in t or "audited" in t,
 "completed": "completed" in t,
}

for k,v in checks.items(): print(("PASS" if v else "FAIL"),k)
print("exit",code)
open("/tmp/tui_goal_capture.txt","w").write(t)
print("goal_needs_criteria:", "goal needs criteria" in t)
print("armed_status_seen:", "next prompt runs audited" in t)
sys.exit(0 if all(checks.values()) and code==0 else 1)
