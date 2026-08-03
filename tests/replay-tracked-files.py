import os
import json,subprocess,sys,os,concurrent.futures as cf
BIN, ROOT = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
EXTS={".rb",".rake",".gemspec",".js",".mjs",".cjs",".jsx",".ts",".mts",".cts",".tsx",
      ".py",".pyi",".go",".rs",".php",".vue",".erb",".rhtml",".toml",".yml",".yaml",
      ".json",".md",".scss",".css",".html",".stub",".rst",".adoc"}
def check(args):
    cwd,rel=args
    try:
        content=open(os.path.join(cwd,rel),encoding="utf-8",errors="replace").read()
    except OSError:
        return None
    if len(content)>400_000: return None
    payload=json.dumps({"session_id":"replay","cwd":cwd,"tool_name":"Write",
        "tool_input":{"file_path":os.path.join(cwd,rel),"content":content}})
    r=subprocess.run([BIN,"inject"],input=payload,capture_output=True,text=True,cwd=cwd)
    if r.returncode!=0: return ("EXIT",rel,r.returncode,r.stderr[:200])
    if r.stderr.strip(): return ("STDERR",rel,0,r.stderr[:200])
    try: d=json.loads(r.stdout or "{}")
    except Exception: return ("BADJSON",rel,0,r.stdout[:200])
    h=d.get("hookSpecificOutput",{})
    if h.get("permissionDecision")=="deny":
        return ("DENY",rel,0,h["permissionDecisionReason"].split("\n\n")[1] if "\n\n" in h["permissionDecisionReason"] else "")
    return None
grand_files=0; grand_bad=[]
for repo in sorted(os.listdir(os.path.join(ROOT,"realrepos"))):
    cwd=os.path.join(ROOT,"realrepos",repo)
    if not os.path.isdir(cwd): continue
    tracked=subprocess.run(["git","ls-files"],cwd=cwd,capture_output=True,text=True).stdout.split("\n")
    files=[f for f in tracked if f and os.path.splitext(f)[1] in EXTS]
    with cf.ThreadPoolExecutor(max_workers=12) as ex:
        results=[r for r in ex.map(check,[(cwd,f) for f in files]) if r]
    grand_files+=len(files); grand_bad+=[(repo,)+r for r in results]
    print(f"  {repo:<14} {len(files):>5} files   {len(results)} refused/failed")
print(f"\nTOTAL {grand_files} tracked files replayed through the write path, {len(grand_bad)} refused or errored")
for b in grand_bad[:60]: print("  ", b[0], b[1], b[2], "|", b[4].strip()[:150])
