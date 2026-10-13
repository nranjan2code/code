import json,re,sys,urllib.request,os
# Downloads ElevenLabs flow results into docs/brand/media. Usage:
#   python3 -I fetch_flow_results.py dl <saved creative_get_flow_run_status result>...
# MAP below ties each generation session id to its path (see manifest.json).
MAP={}
def add(base,ids,ext=None):
    for i,s in enumerate(ids,1): MAP[s]=(base,i,len(ids))
add("sounds/ui/brand-chime",["eIEOp6f7fHK6FXFceMHe","Eo69DXq1WU0xvI86N7GZ","PoEa5zCOTBHsMze6HqUo","Rv2GNNm2krlhwnWfUw28"])
add("sounds/ui/turn-finished",["u0Fw4ZcOhbAN1W6CqX7q","EqTyphuSTGUeCNhZO4eR","2jSo66QtnjeUoDKoeZSd","8PFTR2e26EwN1wTbJZPg"])
add("sounds/ui/approval-needed",["Bh5RdGC6sx6hrXtZJuSW","DxhONb2g7Qk5zdTXH5AF","0XmEwqksh1eHoUGBCI4J","UB18RvXNeJLt5gPwNYV6"])
add("images/hero",["05WSfS8bAgEW79BvWOUM","kOkSZ5gyBVcwmd2FPXRH","b1huitEPajLVjyp3YTVk","EKj3vlURl0dW5MJN4OfR"])
add("video/vakyartha-idle",["6JPYRDN5MCTLFr27bTxY"])
add("music/theme",["g99NCngi9iU5ixCdUmE0","xhkLxPH1IvKcUXxgtBpY"])
for c,ids in {"mira":["JCo7oxIZHmUj1s4Zj2LI","LLpFGR3cYW7xccVh4Wz5"],"moss":["dUTt7PGXqALvh4Ur61q1","3iwNsUJlkUg6odY1BA8I"],"nori":["oestdFTAXz106fZhJJYX","q1Enqis67zUTwdbYxvbC"],"pip":["fFOAUvvzWZs58eBcg1DZ","yHdhrhUkqHxQM1etKLoC"],"lumi":["jT4HipHnuY31h2wY6ij3","z06MqsrpMHE4KmlTgCoA"],"tavi":["7Ls5hOON1RAc2m5CanNA","hqprBzBlUfrQ3dor1iJj"],"beni":["q6ccQ8X4gOf3bRkAj86G","sWBJh3FWlLgDZVmSHBAS"]}.items(): add(f"sounds/companions/{c}",ids)
E=["notification","turn-finished","approval-needed","error","success","listening"]
F={"glass":"uXXwtSjTgGrYzaRakV2N zN8sriP4U8PxAi9qFNxO PfvbjP07iIsCJ4xaanii XJoTJaGhKUyuEFl49E4Z vu98v1a0GZOQmbqIhlvg dVO4xgPJqdc8vyM60ADI",
"santoor":"5KXUCEkKGMGuIx6F29hJ GFqpUWUtnvOoT4lS3kwT QL1YazOb3VR3jl3AsIzV 4NLHwfM8KHJc5CxoRsCt wZ9WEXgqfjfaWLdnGewZ lNFHE6K4SJ84u6aFCzze",
"brass":"WIESF9pjVQZeiyKFyLUq YyqJfyIno7lxxDZbWYj9 oxsxyRioYzwJY4fyizph 3noIx4MvG8nfJooM7HBS 0FVjWDTgvqSEF1lZqsEQ 7AzlfQo7WVSU441D4l1V",
"wood":"ovDG9tmk8ALT9GWSIGJ3 nLKoXe0w7XLu9XXZugW9 CxoHUN1D8kPdS7ckoZ89 fTEX0uVbhUuRlvDZyL6B CkMGREBnLR2AxRqqLnCF ET3A8aBErBNLSpOICGNn",
"pure":"tc7eYgeEzhqztmU6phTg u4dfFo4ry8i58RueChhd iPN3r16iKJvmzdnZ1gZ4 KFxfTTxHVByrYCWghTzT 0bKG0SzowI3PqgVxZ1RT TZqsr1MpoMfz2E1xE9TA"}
for f,s in F.items():
    for e,sid in zip(E,s.split()): MAP[sid]=(f"sounds/system/{f}/{e}",1,1)
if sys.argv[1]=="ids":
    print(json.dumps([s for s in MAP if MAP[s][0].startswith(sys.argv[2])])); sys.exit()
root=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
for path in sys.argv[2:]:
    d=json.load(open(path))
    for m in d.get("media",[]):
        url=m["url"]; sid=re.search(r"content_generation/([^/]+)/",url).group(1)
        if sid not in MAP: print("unmapped",sid); continue
        base,i,n=MAP[sid]; ext=os.path.splitext(url.split("?")[0])[1]
        out=f"{root}/{base}{'-'+str(i) if n>1 else ''}{ext}"
        os.makedirs(os.path.dirname(out),exist_ok=True)
        try: urllib.request.urlretrieve(url,out); print("ok",out[len(root)+1:],os.path.getsize(out))
        except Exception as ex: print("FAIL",base,type(ex).__name__)
