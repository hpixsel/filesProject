"""Copies the icons of Material Icon Theme (VS Code, MIT) that its mapping uses
into crates/files-ui-gpui/assets/material, with map.txt: one
`kind<TAB>key<TAB>icon` line each, kind e (extension), n (file name) or
d (folder name); icon is the SVG's file stem.

Usage: npm pack material-icon-theme, unpack it, then
    python -I tools/material_icons.py <unpacked>/package
"""

import json
import os
import shutil
import sys

pkg = sys.argv[1]
out = os.path.join(os.path.dirname(__file__), "..", "crates", "files-ui-gpui", "assets", "material")
m = json.load(open(os.path.join(pkg, "dist", "material-icons.json"), encoding="utf-8"))
stem = {k: os.path.basename(v["iconPath"])[:-4] for k, v in m["iconDefinitions"].items()}
lines, used = [], {stem[m["file"]], stem[m["folder"]]}
for kind, table in (("e", "fileExtensions"), ("n", "fileNames"), ("d", "folderNames")):
    for key, icon in sorted(m[table].items()):
        if icon in stem and "\t" not in key:
            lines.append(f"{kind}\t{key.lower()}\t{stem[icon]}")
            used.add(stem[icon])
shutil.rmtree(out, ignore_errors=True)
os.makedirs(out)
for name in sorted(used):
    shutil.copy(os.path.join(pkg, "icons", name + ".svg"), out)
shutil.copy(os.path.join(pkg, "LICENSE"), os.path.join(out, "LICENSE"))
with open(os.path.join(out, "map.txt"), "w", encoding="utf-8", newline="\n") as f:
    f.write(f"file\t{stem[m['file']]}\nfolder\t{stem[m['folder']]}\n" + "\n".join(lines) + "\n")
print(len(used), "icons,", len(lines), "names")
