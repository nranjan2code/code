import json,os,subprocess,sys
# Adds batch 5 (companion lines, remaining system sounds) to manifest.json,
# with each take's measured length and peak level. Run after the downloads.
root=os.path.join(os.path.dirname(os.path.abspath(__file__)),"..")
def probe(path):
    p=os.path.join(root,path)
    if not os.path.exists(p): return None
    sec=float(subprocess.run(["ffprobe","-v","error","-show_entries","format=duration","-of","csv=p=0",p],capture_output=True,text=True).stdout.strip() or 0)
    vol=subprocess.run(["ffmpeg","-hide_banner","-i",p,"-af","volumedetect","-f","null","-"],capture_output=True,text=True).stderr
    peak=next((float(l.split(":")[1].split()[0]) for l in vol.splitlines() if "max_volume" in l),None)
    return {"file":path,"seconds":round(sec,1),"peak_db":peak}
PROMPTS={
 ("brass","new-email"):"Small brass temple bell, two light quick strikes, same pitch, soft, close-mic",
 ("brass","reminder"):"Small brass bell, three gentle repeating soft strikes, patient, close-mic",
 ("brass","meeting-soon"):"Small brass bells, rising four-note phrase of soft strikes, warm, brief ring, close-mic",
 ("brass","scheduled-finished"):"Brass bell, soft low strike then high strike, settled and complete, close-mic",
 ("wood","new-email"):"Kalimba, two light quick plucked notes, same pitch, soft, close-mic, dry",
 ("wood","reminder"):"Kalimba, three gentle repeating plucked notes, soft and patient, close-mic, dry",
 ("wood","meeting-soon"):"Kalimba, rising four-note plucked phrase, warm, brief ring, close-mic, dry",
 ("wood","scheduled-finished"):"Kalimba, soft low plucked note then high note, settled and complete, close-mic, dry",
 ("pure","new-email"):"Two soft sine tones, same pitch, quick and light, rounded attack, clean digital",
 ("pure","reminder"):"Three gentle repeating soft sine tones, patient, rounded, clean digital",
 ("pure","meeting-soon"):"Rising four-note phrase of soft sine tones, warm, brief, rounded, clean digital",
 ("pure","scheduled-finished"):"Soft low sine tone then high sine tone, settled and complete, rounded, clean digital",
 ("wood","listening"):"Kalimba, single very short soft rising plucked tap, clearly audible, close-mic, dry",
 ("glass","message-received"):"Glass marimba, single bright high note, quick light tap, short clean decay, close-mic, dry",
 ("santoor","message-received"):"Santoor, single very short high soft struck note, close-mic, dry",
 ("brass","message-received"):"Tiny brass bell, single very short high soft tick, close-mic, dry",
 ("wood","message-received"):"Kalimba, single very short high soft plucked tine, close-mic, dry",
 ("pure","message-received"):"Single very short high soft sine tick, rounded, clean digital",
 ("glass","listening-stop"):"Glass marimba, single soft falling tap, very short, close-mic, dry",
 ("santoor","listening-stop"):"Santoor, single very short soft falling struck note, close-mic, dry",
 ("brass","listening-stop"):"Tiny brass bell, single very short soft low tap, damped, close-mic, dry",
 ("wood","listening-stop"):"Kalimba, single very short soft low plucked note, damped, close-mic, dry",
 ("pure","listening-stop"):"Very short soft sine blip, falling pitch, rounded, clean digital",
}
lines=json.load(open(os.path.join(root,"companion-lines.json")))
m=json.load(open(os.path.join(root,"manifest.json")))
m["assets"]=[a for a in m["assets"] if a.get("batch")!=5]
for line in open(os.path.join(root,"tools","batch5_sessions.txt")):
    if line.startswith("#") or not line.strip(): continue
    kind,who,moment,*ids=line.split()
    if kind=="speech":
        c=lines["companions"][who]
        takes=[probe(f"speech/en/{who}/{moment}-{i}.mp3") for i in (1,2)]
        a={"batch":5,"kind":"announcement" if moment.startswith("announce-") else "speech","character":who,
           "moment":moment,"voice_id":c["voice_id"],"model":"eleven_v4","language":"en","text":c["lines"][moment]}
    else:
        takes=[probe(f"sounds/system/{who}/{moment}-{i}.mp3") for i in (1,2)]
        a={"batch":5,"kind":"system sound","family":who,"event":moment,"model":"eleven_text_to_sound_v2",
           "prompt":PROMPTS[(who,moment)],"settings":{"duration_seconds":1.5 if moment in ("new-email","reminder","meeting-soon","scheduled-finished") else 1.0,"prompt_influence":0.5}}
    takes=[t for t in takes if t]
    silent=[t for t in takes if t["peak_db"] is not None and t["peak_db"]<-40]
    a["takes"]=takes
    a["status"]="downloaded" if len(takes)==2 and not silent else f"check: {len(takes)} takes, {len(silent)} silent"
    m["assets"].append(a)
for a in m["assets"]:
    if a.get("status")=="silent" and a.get("family")=="wood" and a.get("event")=="listening":
        a["note"]+=" Batch 5 made two new takes (listening-1, listening-2)."
json.dump(m,open(os.path.join(root,"manifest.json"),"w"),indent=2,ensure_ascii=False); open(os.path.join(root,"manifest.json"),"a").write("\n")
b=[a for a in m["assets"] if a.get("batch")==5]
print(len(b),"assets;",sum(len(a["takes"]) for a in b),"takes;",[ (a.get("character") or a.get("family"),a.get("moment") or a.get("event"),a["status"]) for a in b if a["status"]!="downloaded"])
