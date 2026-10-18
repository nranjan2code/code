"""The plant diagram: every subsystem as a unit in its plant area, every
measured connection as a pipe, and every flow as a route a packet travels.

Drawn as static SVG from the model, so the page is complete at rest; the
script in template.html only animates and filters what is already drawn.
The areas' arrangement is the one hand-made part (`ROWS`); a group missing
from it is reported.
"""

import html
import json

# Plant areas, row by row: (group, columns of units).
ROWS = [
    [("Surfaces", 2), ("Service and operations", 4), ("Presentation", 2)],
    [("The turn", 3), ("Models and tools", 3), ("Safety", 2)],
    [("Apps", 4), ("Data", 5)],
]
NODE_W, NODE_H, GAP_X, GAP_Y = 172, 64, 14, 14
FIT = 20  # characters of the unit's label font that fit on one line
PAD, HEAD, AREA_GAP, MARGIN = 16, 34, 30, 24


def esc(s):
    return html.escape(str(s))


def label_lines(name):
    """The unit's name on one line, or split at the space nearest its middle."""
    if len(name) <= FIT or " " not in name:
        return [name]
    spaces = [i for i, ch in enumerate(name) if ch == " "]
    cut = min(spaces, key=lambda i: abs(i - len(name) / 2))
    return [name[:cut], name[cut + 1:]]


def layout(model):
    """{subsystem id: (x, y)} of each unit's top-left, the areas, the canvas
    size, and review lines."""
    review = []
    by_group = {}
    for x in model["subsystems"]:
        by_group.setdefault(x["group"], []).append(x)
    placed = [g for row in ROWS for g, _ in row]
    review += [f"plant: group {g} is in no row (plant.ROWS)" for g in by_group if g not in placed]
    review += [f"plant: row names unknown group {g}" for g in placed if g not in by_group]
    rows = []
    for row in ROWS:
        areas = []
        for group, cols in row:
            items = by_group.get(group, [])
            n_rows = max(1, -(-len(items) // cols))
            w = PAD * 2 + cols * NODE_W + (cols - 1) * GAP_X
            h = HEAD + PAD + n_rows * NODE_H + (n_rows - 1) * GAP_Y + PAD
            areas.append({"group": group, "cols": cols, "w": w, "h": h, "items": items})
        rows.append(areas)
    width = max(sum(a["w"] for a in r) + AREA_GAP * (len(r) - 1) for r in rows) + 2 * MARGIN
    pos, drawn, y = {}, [], MARGIN
    for r in rows:
        row_w = sum(a["w"] for a in r) + AREA_GAP * (len(r) - 1)
        row_h = max(a["h"] for a in r)
        x = (width - row_w) / 2
        for a in r:
            drawn.append({"group": a["group"], "x": x, "y": y, "w": a["w"], "h": row_h})
            for i, item in enumerate(a["items"]):
                cx = x + PAD + (i % a["cols"]) * (NODE_W + GAP_X)
                cy = y + HEAD + PAD + (i // a["cols"]) * (NODE_H + GAP_Y)
                pos[item["id"]] = (cx, cy)
            x += a["w"] + AREA_GAP
        y += row_h + AREA_GAP
    return pos, drawn, (width, y - AREA_GAP + MARGIN), review


def pipe(pos, a, b):
    """An S-shaped pipe from unit a's nearest side to unit b's."""
    ax, ay = pos[a]
    bx, by = pos[b]
    acx, acy, bcx, bcy = ax + NODE_W / 2, ay + NODE_H / 2, bx + NODE_W / 2, by + NODE_H / 2
    if abs(bcx - acx) >= abs(bcy - acy) * 1.6:
        sx, ex = (ax + NODE_W, bx) if bcx > acx else (ax, bx + NODE_W)
        mid = (sx + ex) / 2
        return f"M{sx:.0f},{acy:.0f} C{mid:.0f},{acy:.0f} {mid:.0f},{bcy:.0f} {ex:.0f},{bcy:.0f}"
    sy, ey = (ay + NODE_H, by) if bcy > acy else (ay, by + NODE_H)
    mid = (sy + ey) / 2
    return f"M{acx:.0f},{sy:.0f} C{acx:.0f},{mid:.0f} {bcx:.0f},{mid:.0f} {bcx:.0f},{ey:.0f}"


def svg(model):
    pos, areas, (W, H), review = layout(model)
    top_recent = max([x["recent"] for x in model["subsystems"]] + [1])
    out = [f'<svg class="plant-svg" viewBox="0 0 {W:.0f} {H:.0f}" role="img" '
           f'aria-label="Plant diagram: {len(model["subsystems"])} subsystems in {len(areas)} areas, '
           f'{len(model["connections"])} measured connections, and the flows that move through them">',
           '<defs><marker id="pa" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">'
           '<path d="M0,0L10,5L0,10z" class="pa"/></marker>'
           '<pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">'
           '<line x1="0" y1="0" x2="0" y2="6" class="hatch"/></pattern></defs>']
    for a in areas:
        out.append(f'<g class="area"><rect x="{a["x"]:.0f}" y="{a["y"]:.0f}" width="{a["w"]:.0f}" height="{a["h"]:.0f}"/>'
                   f'<rect class="area-tab" x="{a["x"]:.0f}" y="{a["y"]:.0f}" width="{a["w"]:.0f}" height="{HEAD - 6}"/>'
                   f'<text class="area-name" x="{a["x"] + 12:.0f}" y="{a["y"] + 20:.0f}">{esc(a["group"].upper())}</text></g>')
    out.append('<g class="pipes">')
    for e in model["connections"]:
        if e["from"] in pos and e["to"] in pos:
            out.append(f'<path class="pipe {e["kind"]}" data-from="{e["from"]}" data-to="{e["to"]}" data-w="{e["weight"]}" '
                       f'd="{pipe(pos, e["from"], e["to"])}"><title>{esc(e["from"])} → {esc(e["to"])}: '
                       f'{e["weight"]} {"route call" if e["kind"] == "http" else "code reference"}{"s" if e["weight"] != 1 else ""}</title></path>')
    out.append('</g><g class="flowpipes">')
    for f in model["flows"]:
        for i, s in enumerate(f["steps"]):
            if s["from"] in pos and s["to"] in pos:
                out.append(f'<path class="fstep" data-flow="{f["id"]}" data-i="{i}" marker-end="url(#pa)" '
                           f'd="{pipe(pos, s["from"], s["to"])}"/>')
    out.append('</g><g class="units">')
    for x in model["subsystems"]:
        if x["id"] not in pos:
            continue
        px, py = pos[x["id"]]
        load = x["recent"] / top_recent
        lines = label_lines(x["name"])
        out.append(f'<g class="unit" data-id="{x["id"]}" tabindex="0" role="button" aria-label="{esc(x["name"])}: '
                   f'{x["lines"]:,} lines, {x["commits"]} commits, {len(x["out"]) + len(x["in"])} connections">'
                   f'<rect class="u-body" x="{px:.0f}" y="{py:.0f}" width="{NODE_W}" height="{NODE_H}" rx="3"/>'
                   f'<rect class="u-load" x="{px + 1:.0f}" y="{py + NODE_H - 6:.0f}" width="{(NODE_W - 2) * load:.1f}" height="5"/>'
                   f'<circle class="u-led{" live" if x["recent"] else ""}" cx="{px + NODE_W - 12:.0f}" cy="{py + 13:.0f}" r="4.5"/>'
                   + "".join(f'<text class="u-name" x="{px + 10:.0f}" y="{py + (19 + 16 * i if len(lines) > 1 else 25):.0f}">{esc(t)}</text>'
                             for i, t in enumerate(lines))
                   + f'<text class="u-sub" x="{px + 10:.0f}" y="{py + (51 if len(lines) > 1 else 45):.0f}">{x["lines"] / 1000:.1f}k · {x["commits"]}c · {len(x["routes"])}r</text>'
                   f'</g>')
    out.append('</g><g class="packets"></g></svg>')
    return "\n".join(out), review


def data(model):
    """What the script needs and the SVG does not carry, as JSON."""
    att = model["attention"]
    return json.dumps({
        "units": {x["id"]: {"name": x["name"], "group": x["group"], "role": x["role"], "lines": x["lines"],
                            "commits": x["commits"], "recent": x["recent"], "fixes": x["fixes"],
                            "homes": x["homes"], "routes": len(x["routes"]), "cli": x["cli"],
                            "out": [[e["to"], e["kind"], e["weight"]] for e in x["out"]],
                            "in": [[e["from"], e["kind"], e["weight"]] for e in x["in"]]}
                  for x in model["subsystems"]},
        "flows": model["flows"],
        "replay": list(reversed(model["replay"])),
        "kpi": {"subsystems": len(model["subsystems"]), "connections": len(model["connections"]),
                "flows": len(model["flows"]), "routes": model["summary"]["stats"].get("routes", 0),
                "checks_ok": sum(1 for c in att["health"] if c["ok"]), "checks": len(att["health"]),
                "open": att["open_total"], "recent": len(model["replay"]),
                "since": model["activity"]["since"], "until": model["activity"]["until"],
                "revision": model["meta"].get("revision", ""), "version": model["meta"].get("version", "")},
    }, ensure_ascii=False).replace("</", "<\\/")


def flow_buttons(model):
    return "\n".join(f'      <button type="button" data-flow="{f["id"]}" aria-pressed="false">{esc(f["name"])}</button>'
                     for f in model["flows"])


def flows_html(model):
    names = {x["id"]: x["name"] for x in model["subsystems"]}
    out = ['<div class="flowlist">']
    for f in model["flows"]:
        out.append(f'<div class="flow"><h3><button type="button" class="linkish" data-goto-flow="{f["id"]}">{esc(f["name"])}</button></h3><ol>')
        out += [f'<li><b>{esc(names.get(s["from"], s["from"]))}</b> → <b>{esc(names.get(s["to"], s["to"]))}</b>: {esc(s["what"])}</li>'
                for s in f["steps"]]
        out.append("</ol></div>")
    out.append("</div>")
    return "\n".join(out)


def connections_html(model):
    names = {x["id"]: x["name"] for x in model["subsystems"]}
    out = ['<div class="scroll"><table class="subs"><thead><tr><th>Subsystem</th><th class="n">Out</th><th class="n">In</th>'
           '<th>Uses (code references · route calls)</th><th>Used by</th></tr></thead><tbody>']

    def items(rows, key):
        return ", ".join(f'{esc(names.get(r[key], r[key]))}&nbsp;<span class="muted">{r["weight"]}{" http" if r["kind"] == "http" else ""}</span>'
                         for r in rows) or "—"
    for x in sorted(model["subsystems"], key=lambda s: -(len(s["out"]) + len(s["in"]))):
        out.append(f'<tr><td><b>{esc(x["name"])}</b></td><td class="n">{len(x["out"])}</td><td class="n">{len(x["in"])}</td>'
                   f'<td>{items(x["out"], "to")}</td><td>{items(x["in"], "from")}</td></tr>')
    out.append("</tbody></table></div>")
    return "\n".join(out)
