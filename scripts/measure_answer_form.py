# Usage: python3 scripts/measure_answer_form.py <ollama-model> <trials> <out.json>
# Measures, on the local dev route, whether one nested answer document is
# written as reliably as flat per-block tools. Same block fields both ways.
import json,urllib.request,time,sys
MODEL=sys.argv[1]; N=int(sys.argv[2])
BLOCK={ "stats":{"label":{"type":"string","description":"A short heading."},"figures":{"type":"array","items":{"type":"string"},"description":"Each figure as 'Label | value'."}},
        "steps":{"label":{"type":"string","description":"A short heading."},"steps":{"type":"array","items":{"type":"string"},"description":"Each step as one sentence."}},
        "callout":{"tone":{"type":"string","enum":["tip","note","warning"]},"text":{"type":"string","description":"The note."}}}
REQ={"stats":["figures"],"steps":["steps"],"callout":["tone","text"]}
def tool(name,desc,props,req): return {"type":"function","function":{"name":name,"description":desc,"parameters":{"type":"object","properties":props,"required":req}}}
FLAT=[tool("show_"+k,"Show a %s block to the person."%k,v,REQ[k]) for k,v in BLOCK.items()]
allp={"type":{"type":"string","enum":list(BLOCK),"description":"The block's kind."}}
for v in BLOCK.values(): allp.update(v)
NEST=[tool("answer","Show the whole answer to the person as an ordered list of blocks.",{"blocks":{"type":"array","description":"The answer's blocks, in order.","items":{"type":"object","properties":allp,"required":["type"]}}},["blocks"])]
PROMPTS=["I have 3 tomato seedlings, 2 basil pots and 1 lemon sapling. Show the counts as stats, then a four-step watering routine as steps, then one warning about overwatering as a callout.",
         "My trip costs: flights 420, hotel 380, food 150. Show those as stats, then the steps to book in order, then a tip about travel insurance as a callout."]
SYS="You answer by calling the tools you are given. Do not write the answer as text."
TEXT_LISTS=[0]
def read(b):
    # what vak_tools::read_arguments does: a list sent as its JSON text is its items
    for k in ("figures","steps"):
        v=b.get(k)
        if isinstance(v,str):
            try:
                d=json.loads(v.strip())
                if isinstance(d,list): b[k]=d; TEXT_LISTS[0]+=1
            except Exception: pass
    return b
def ok_block(kind,b):
    b=read(b)
    if kind=="stats": return isinstance(b.get("figures"),list) and len(b["figures"])>=2 and all(isinstance(x,str) for x in b["figures"])
    if kind=="steps": return isinstance(b.get("steps"),list) and len(b["steps"])>=2 and all(isinstance(x,str) for x in b["steps"])
    if kind=="callout": return isinstance(b.get("text"),str) and b.get("tone") in ("tip","note","warning")
    return False
def chat(tools,prompt):
    body={"model":MODEL,"stream":False,"think":False,"messages":[{"role":"system","content":SYS},{"role":"user","content":prompt}],"tools":tools}
    r=urllib.request.Request("http://127.0.0.1:11434/api/chat",data=json.dumps(body).encode(),headers={"Content-Type":"application/json"})
    try: return json.load(urllib.request.urlopen(r,timeout=600))["message"]
    except Exception as e: return {"error":str(e)[:80]}
res={"flat":[], "nested":[]}
for i in range(N):
    for p in PROMPTS:
        m=chat(FLAT,p); calls=m.get("tool_calls") or []
        got={}; bad=[]
        for c in calls:
            n=c["function"]["name"].replace("show_",""); a=c["function"].get("arguments")
            if isinstance(a,dict) and n in BLOCK and ok_block(n,a): got[n]=True
            elif isinstance(a,dict): bad.append((n,{k:type(v).__name__ for k,v in a.items()}))
        res["flat"].append({"calls":len(calls),"valid":sorted(got),"all":len(got)==3,"err":m.get("error"),"bad":bad})
        m=chat(NEST,p); calls=m.get("tool_calls") or []
        got={}; shape="none"; bad=[]
        if calls:
            a=calls[0]["function"].get("arguments"); bl=a.get("blocks") if isinstance(a,dict) else None
            shape=type(bl).__name__
            if isinstance(bl,list):
                for b in bl:
                    if isinstance(b,dict) and ok_block(b.get("type"),b): got[b["type"]]=True
                    elif isinstance(b,dict): bad.append((b.get("type"),{k:type(v).__name__ for k,v in b.items()}))
        res["nested"].append({"calls":len(calls),"blocks":shape,"valid":sorted(got),"all":len(got)==3,"err":m.get("error"),"bad":bad})
        json.dump(res,open(sys.argv[3],"w"))
for k,v in res.items():
    print(k,"trials",len(v),"all three valid",sum(x["all"] for x in v),"no call",sum(x["calls"]==0 for x in v),"errors",sum(bool(x["err"]) for x in v))
print("lists sent as JSON text and read:",TEXT_LISTS[0])
for k,v in res.items(): print(k,"invalid blocks:",[x["bad"] for x in v if x["bad"]][:6])
print("nested blocks shapes",[x["blocks"] for x in res["nested"]])
