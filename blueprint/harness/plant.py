"""The plant diagram: every subsystem as a unit on the layer stack's layers,
coloured by plant area, what lies outside the repository on L0, every
measured connection as a pipe, and every flow as a route a packet travels.

Drawn as static SVG from the model, so the page is complete at rest; the
script in template.html only animates and filters what is already drawn.
"""

import html
import json

# The plant is drawn on the layer stack's bands, top to bottom, so the two
# sheets read the same way: each subsystem on the layer of the crate that
# holds most of its code (model.py), and what lies outside the repository on
# L0, as the layer stack draws it. The plant areas are the units' colours.
PER_LINE = 8
NODE_W, NODE_H, GAP_X, GAP_Y = 172, 64, 14, 14
FIT = 20  # characters of the unit's label font that fit on one line
LABEL_W, PAD, BAND_GAP, MARGIN = 196, 14, 16, 18
L0_NAME = "Host and outside"


def esc(s):
    return html.escape(str(s))


def label_lines(name):
    """The unit's name on one line, or split at the space nearest its middle."""
    if len(name) <= FIT or " " not in name:
        return [name]
    spaces = [i for i, ch in enumerate(name) if ch == " "]
    cut = min(spaces, key=lambda i: abs(i - len(name) / 2))
    return [name[:cut], name[cut + 1:]]


def bands(model):
    """The layer bands top to bottom, each with its units: subsystems in plant
    area order, then the parts outside the repository."""
    group_order = {g: i for i, g in enumerate(model["groups"])}
    out = []
    for layer in [l for l in model["layers"] if l["id"] != "L?"] + [{"id": "L0", "name": L0_NAME}]:
        units = sorted((x for x in model["subsystems"] if x["layer"] == layer["id"]),
                       key=lambda x: (group_order.get(x["group"], 99), -x["lines"]))
        items = [{"kind": "sub", **x} for x in units]
        if layer["id"] == "L0":
            items += [{"kind": "ext", **e} for e in model["externals"]]
        out.append({"id": layer["id"], "name": layer["name"], "items": items,
                    "lines": sum(x["lines"] for x in units), "subs": len(units)})
    return out


def layout(model):
    """{unit id: (x, y)}, the bands as drawn, and the canvas size."""
    pos, drawn, y = {}, [], MARGIN
    width = MARGIN * 2 + LABEL_W + PAD * 2 + PER_LINE * NODE_W + (PER_LINE - 1) * GAP_X
    for b in bands(model):
        n_lines = max(1, -(-len(b["items"]) // PER_LINE))
        h = PAD * 2 + n_lines * NODE_H + (n_lines - 1) * GAP_Y
        drawn.append({**b, "y": y, "h": h})
        for i, item in enumerate(b["items"]):
            x0 = MARGIN + LABEL_W + PAD + (i % PER_LINE) * (NODE_W + GAP_X)
            pos[item["id"]] = (x0, y + PAD + (i // PER_LINE) * (NODE_H + GAP_Y))
        y += h + BAND_GAP
    return pos, drawn, (width, y - BAND_GAP + MARGIN)


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
    pos, drawn, (W, H) = layout(model)
    group_index = {g: i + 1 for i, g in enumerate(model["groups"])}
    top_recent = max([x["recent"] for x in model["subsystems"]] + [1])
    out = [f'<svg class="plant-svg" viewBox="0 0 {W:.0f} {H:.0f}" role="img" '
           f'aria-label="Plant diagram on the layer stack: {len(model["subsystems"])} subsystems and '
           f'{len(model["externals"])} outside parts on {len(drawn)} layers, {len(model["connections"])} measured connections, '
           f'and the flows that move through them">',
           '<defs><marker id="pa" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">'
           '<path d="M0,0L10,5L0,10z" class="pa"/></marker>'
           '<filter id="pglow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="3" result="b"/>'
           '<feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>']
    for b in drawn:
        sub = f'{b["subs"]} subsystem{"s" if b["subs"] != 1 else ""}' + (f' · {b["lines"] / 1000:.0f}k lines' if b["lines"] else "")
        if b["id"] == "L0":
            sub = f'{len(model["externals"])} outside parts' + (f' · {b["subs"]} subsystem' if b["subs"] else "")
        out.append(f'<g class="band-row{" ext" if b["id"] == "L0" else ""}" data-layer="{b["id"]}">'
                   f'<rect class="band" x="{MARGIN}" y="{b["y"]:.0f}" width="{W - 2 * MARGIN:.0f}" height="{b["h"]:.0f}"/>'
                   f'<text class="lid" x="{MARGIN + 14}" y="{b["y"] + 26:.0f}">{b["id"]}</text>'
                   f'<text class="lname" x="{MARGIN + 14}" y="{b["y"] + 47:.0f}">{esc(b["name"])}</text>'
                   f'<text class="lsub" x="{MARGIN + 14}" y="{b["y"] + 64:.0f}">{esc(sub)}</text></g>')
    out.append('<g class="pipes">')
    for e in model["connections"]:
        if e["from"] in pos and e["to"] in pos:
            out.append(f'<path class="pipe {e["kind"]}" data-from="{e["from"]}" data-to="{e["to"]}" '
                       f'd="{pipe(pos, e["from"], e["to"])}"/>')
    for x in model["externals"]:
        for sid in x["reach"]:
            if sid in pos and x["id"] in pos:
                out.append(f'<path class="pipe outside" data-from="{sid}" data-to="{x["id"]}" d="{pipe(pos, sid, x["id"])}"/>')
    out.append('</g><g class="flowpipes">')
    for f in model["flows"]:
        for i, s in enumerate(f["steps"]):
            if s["from"] in pos and s["to"] in pos:
                out.append(f'<path class="fstep" data-flow="{f["id"]}" data-i="{i}" marker-end="url(#pa)" '
                           f'd="{pipe(pos, s["from"], s["to"])}"/>')
    out.append('</g><g class="units">')
    for b in drawn:
        for item in b["items"]:
            px, py = pos[item["id"]]
            lines = label_lines(item["name"])
            name_y = [py + (20 + 16 * i if len(lines) > 1 else 27) for i in range(len(lines))]
            names = "".join(f'<text class="u-name" x="{px + 14:.0f}" y="{y:.0f}">{esc(t)}</text>' for t, y in zip(lines, name_y))
            if item["kind"] == "ext":
                out.append(f'<g class="unit ext" data-id="{item["id"]}" tabindex="0" role="button" aria-label="{esc(item["name"])}, outside the repository">'
                           f'<rect class="u-body" x="{px:.0f}" y="{py:.0f}" width="{NODE_W}" height="{NODE_H}" rx="10"/>{names}'
                           f'<text class="u-sub" x="{px + 14:.0f}" y="{py + (52 if len(lines) > 1 else 46):.0f}">outside · {len(item["reach"])} link{"s" if len(item["reach"]) != 1 else ""}</text></g>')
                continue
            x = item
            g = group_index.get(x["group"], 0)
            load = x["recent"] / top_recent
            out.append(f'<g class="unit g{g}" data-id="{x["id"]}" data-group="{esc(x["group"])}" data-homes="{esc(" ".join(x["homes"]))}" '
                       f'tabindex="0" role="button" aria-label="{esc(x["name"])}: {x["lines"]:,} lines, {x["commits"]} commits, '
                       f'{len(x["out"]) + len(x["in"])} connections">'
                       f'<rect class="u-body" x="{px:.0f}" y="{py:.0f}" width="{NODE_W}" height="{NODE_H}" rx="3"/>'
                       f'<rect class="u-stripe" x="{px:.0f}" y="{py:.0f}" width="5" height="{NODE_H}"/>'
                       f'<rect class="u-track" x="{px + 14:.0f}" y="{py + NODE_H - 9:.0f}" width="{NODE_W - 28}" height="3"/>'
                       f'<rect class="u-load" x="{px + 14:.0f}" y="{py + NODE_H - 9:.0f}" width="{(NODE_W - 28) * load:.1f}" height="3"/>'
                       f'<circle class="u-led{" live" if x["recent"] else ""}" cx="{px + NODE_W - 13:.0f}" cy="{py + 13:.0f}" r="4.5"/>'
                       f'{names}<text class="u-sub" x="{px + 14:.0f}" y="{py + (50 if len(lines) > 1 else 44):.0f}">'
                       f'{x["lines"] / 1000:.1f}k · {x["commits"]}c · {len(x["out"]) + len(x["in"])} links</text></g>')
    out.append('</g><g class="packets" filter="url(#pglow)"></g></svg>')
    return "\n".join(out), []


def data(model):
    """What the script needs and the SVG does not carry, as JSON."""
    att = model["attention"]
    units = {x["id"]: {"name": x["name"], "group": x["group"], "layer": x["layer"], "role": x["role"], "lines": x["lines"],
                       "commits": x["commits"], "recent": x["recent"], "fixes": x["fixes"],
                       "homes": x["homes"], "routes": len(x["routes"]), "cli": x["cli"],
                       "out": [[e["to"], e["kind"], e["weight"]] for e in x["out"]],
                       "in": [[e["from"], e["kind"], e["weight"]] for e in x["in"]]}
             for x in model["subsystems"]}
    for e in model["externals"]:
        units[e["id"]] = {"name": e["name"], "group": "Outside the repository", "layer": "L0", "role": e["role"], "external": True,
                          "out": [], "in": [[sid, "outside", 1] for sid in e["reach"]]}
        for sid in e["reach"]:
            if sid in units:
                units[sid]["out"].append([e["id"], "outside", 1])
    return json.dumps({
        "units": units,
        "groups": model["groups"],
        "flows": model["flows"],
        "replay": list(reversed(model["replay"])),
        "kpi": {"subsystems": len(model["subsystems"]), "connections": len(model["connections"]),
                "flows": len(model["flows"]), "routes": model["summary"]["stats"].get("routes", 0),
                "checks_ok": sum(1 for c in att["health"] if c["ok"]), "checks": len(att["health"]),
                "open": att["open_total"], "recent": len(model["replay"]),
                "since": model["activity"]["since"], "until": model["activity"]["until"],
                "revision": model["meta"].get("revision", ""), "version": model["meta"].get("version", "")},
    }, ensure_ascii=False).replace("</", "<\\/")


def area_chips(model):
    return "\n".join(f'      <button type="button" class="area-chip g{i + 1}" data-area="{esc(g)}" aria-pressed="false"><i></i>{esc(g)}</button>'
                     for i, g in enumerate(model["groups"]))


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


def _merged(edges, key):
    """{other subsystem: {"code": n, "http": n}} for one side of a unit."""
    out = {}
    for e in edges:
        out.setdefault(e[key], {"code": 0, "http": 0})[e["kind"]] += e["weight"]
    return sorted(out.items(), key=lambda kv: (-(kv[1]["code"] + kv[1]["http"]), kv[0]))


def connections_html(model):
    """A picker of every subsystem by plant area, and for the one picked, what
    it uses and what uses it: one row per neighbour, its code references and
    route calls as one split bar."""
    subs = {x["id"]: x for x in model["subsystems"]}
    groups = []
    for x in model["subsystems"]:
        if not groups or groups[-1][0] != x["group"]:
            groups.append((x["group"], []))
        groups[-1][1].append(x)
    first = max(model["subsystems"], key=lambda x: len(x["out"]) + len(x["in"]))["id"] if model["subsystems"] else ""
    pick = ['<nav class="cx-pick" aria-label="Subsystems">']
    for group, items in groups:
        pick.append(f'<h4>{esc(group)}</h4>')
        for x in items:
            pick.append(f'<button type="button" data-cx="{x["id"]}" aria-pressed="{"true" if x["id"] == first else "false"}">'
                        f'<span>{esc(x["name"])}</span><span class="cx-n">{len(x["out"])}↗ {len(x["in"])}↙</span></button>')
    pick.append("</nav>")

    def side(title, rows, empty):
        if not rows:
            return f'<div class="cx-side"><h4>{title} (0)</h4><p class="muted">{empty}</p></div>'
        top = max(v["code"] + v["http"] for _, v in rows) or 1
        by_group = {}
        for other, v in rows:
            by_group.setdefault(subs[other]["group"] if other in subs else "", []).append((other, v))
        out = [f'<div class="cx-side"><h4>{title} ({len(rows)})</h4>']
        for group in [g for g, _ in groups if g in by_group]:
            out.append(f'<div class="cx-group">{esc(group)}</div>')
            for other, v in by_group[group]:
                n = subs[other]["name"] if other in subs else other
                parts = [f'{v["code"]} code' if v["code"] else "", f'{v["http"]} route' if v["http"] else ""]
                out.append(f'<div class="cx-row"><button type="button" class="linkish" data-cx="{other}">{esc(n)}</button>'
                           f'<span class="cx-bar" aria-hidden="true"><i class="c" style="width:{100 * v["code"] / top:.1f}%"></i>'
                           f'<i class="h" style="width:{100 * v["http"] / top:.1f}%"></i></span>'
                           f'<span class="cx-v">{" · ".join(p for p in parts if p)}</span></div>')
        out.append("</div>")
        return "".join(out)

    panels = []
    for x in model["subsystems"]:
        panels.append(
            f'<section class="cx-panel" data-cx-panel="{x["id"]}"{"" if x["id"] == first else " hidden"}>'
            f'<div class="cx-head"><div><div class="kicker">{esc(x["group"])}</div><h3>{esc(x["name"])}</h3>'
            f'<p class="muted">{esc(x["role"])}</p></div>'
            f'<button type="button" class="linkish" data-cx-plant="{x["id"]}">Show on the plant diagram</button></div>'
            f'<div class="cx-cols">{side("Uses", _merged(x["out"], "to"), "Names no other subsystem.")}'
            f'{side("Used by", _merged(x["in"], "from"), "No other subsystem names it.")}</div></section>')
    legend = ('<div class="plant-legend"><span><i class="cxc"></i>code references (file pairs)</span>'
              '<span><i class="cxh"></i>route calls (client files naming a route)</span>'
              '<span>Bars compare neighbours of the selected subsystem.</span></div>')
    return f'<div class="cx">{"".join(pick)}<div class="cx-detail">{"".join(panels)}{legend}</div></div>'
