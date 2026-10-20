import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import * as api from "../../api";
import { mapSettings } from "../../mapSettings";
import { Card, num, records, text, type Payload } from "./shared";

/** Places on a map (docs/design/96 P5), drawn with no network: the places
 *  projected with Web Mercator onto a plain surface, a scale and a north
 *  mark. Street tiles sit underneath only while the owner allows them in
 *  Settings; they come through the server, which refuses them otherwise. */

interface Place { label: string; detail: string | undefined; lat: number; lon: number }

const EARTH_M = 40_075_016.686;
const RADIUS_M = 6_371_008.8;
const PAD = 40;
const MARKER = 11;
const LABEL_LINE = 18;
const TILE = 256;

const worldX = (lon: number) => (lon + 180) / 360;
const worldY = (lat: number) => {
  const phi = (Math.max(-85.0511, Math.min(85.0511, lat)) * Math.PI) / 180;
  return (1 - Math.log(Math.tan(phi) + 1 / Math.cos(phi)) / Math.PI) / 2;
};
const latOf = (y: number) => (Math.atan(Math.sinh(Math.PI * (1 - 2 * y))) * 180) / Math.PI;

const rad = (degrees: number) => (degrees * Math.PI) / 180;

/** Great-circle distance in metres. */
function distance(a: Place, b: Place): number {
  const h = Math.sin(rad(b.lat - a.lat) / 2) ** 2
    + Math.cos(rad(a.lat)) * Math.cos(rad(b.lat)) * Math.sin(rad(b.lon - a.lon) / 2) ** 2;
  return 2 * RADIUS_M * Math.asin(Math.min(1, Math.sqrt(h)));
}

/** The area inside a ring of places on the sphere, in square metres. */
function area(ring: Place[]): number {
  let sum = 0;
  ring.forEach((a, i) => {
    const b = ring[(i + 1) % ring.length];
    sum += rad(b.lon - a.lon) * (2 + Math.sin(rad(a.lat)) + Math.sin(rad(b.lat)));
  });
  return Math.abs((sum * RADIUS_M * RADIUS_M) / 2);
}

/** The largest 1, 2 or 5 × 10ⁿ at most `limit`. */
function roundDown(limit: number): number {
  const power = 10 ** Math.floor(Math.log10(limit));
  return [5, 2, 1].map((step) => step * power).find((value) => value <= limit) ?? power;
}

const number = (value: number, digits: number) =>
  new Intl.NumberFormat(undefined, { maximumFractionDigits: digits }).format(value);

function length(metres: number): string {
  if (metres >= 1000) return `${number(metres / 1000, metres >= 100_000 ? 0 : 1)} km`;
  return `${number(metres, 0)} m`;
}

function size(square: number): string {
  if (square >= 1_000_000) return `${number(square / 1_000_000, square >= 100_000_000 ? 0 : 1)} km²`;
  if (square >= 10_000) return `${number(square / 10_000, 1)} ha`;
  return `${number(square, 0)} m²`;
}

interface Box { x: number; y: number; w: number; h: number }
const overlaps = (a: Box, b: Box) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;

export default function MapCard(props: { payload: Payload }) {
  const p = () => props.payload;
  const connect = () => (text(p().connect) === "route" ? "route" : text(p().connect) === "area" ? "area" : "none");
  const places = createMemo<Place[]>(() =>
    records(p().places)
      .map((place) => ({ label: text(place.label) ?? "", detail: text(place.detail), lat: num(place.latitude), lon: num(place.longitude) }))
      .filter((place): place is Place => place.lat !== undefined && place.lon !== undefined),
  );

  let host: HTMLDivElement | undefined;
  const [width, setWidth] = createSignal(640);
  onMount(() => {
    if (!host) return;
    const observer = new ResizeObserver(([entry]) => setWidth(Math.max(1, Math.floor(entry.contentRect.width))));
    observer.observe(host);
    onCleanup(() => observer.disconnect());
  });
  const height = () => Math.round(Math.min(440, Math.max(240, width() * 0.6)));

  /** The projection: world units to pixels, fitted to the places. A single
   *  place, or places at one point, is shown about two kilometres across. */
  const view = createMemo(() => {
    const xs = places().map((place) => worldX(place.lon));
    const ys = places().map((place) => worldY(place.lat));
    const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
    const centreX = (minX + maxX) / 2;
    const centreY = (minY + maxY) / 2;
    const lat = latOf(centreY);
    const across = width() - 2 * PAD;
    const down = height() - 2 * PAD;
    const spread = Math.max(maxX - minX, maxY - minY);
    const scale = spread > 0
      ? Math.min(across / Math.max(maxX - minX, 1e-12), down / Math.max(maxY - minY, 1e-12))
      : across / (2000 / (EARTH_M * Math.cos(rad(lat))));
    const toX = (x: number) => width() / 2 + (x - centreX) * scale;
    const toY = (y: number) => height() / 2 + (y - centreY) * scale;
    const metresPerPixel = (EARTH_M * Math.cos(rad(lat))) / scale;
    return { centreX, centreY, scale, toX, toY, metresPerPixel };
  });

  const points = createMemo(() => places().map((place) => ({ x: view().toX(worldX(place.lon)), y: view().toY(worldY(place.lat)) })));

  /** Each label beside its marker where it fits without covering another
   *  marker or label; every label is in the legend whatever happens here. */
  const labels = createMemo(() => {
    const taken: Box[] = points().map((point) => ({ x: point.x - MARKER, y: point.y - MARKER, w: 2 * MARKER, h: 2 * MARKER }));
    return places().map((place, index) => {
      const point = points()[index];
      const w = place.label.length * 7.6;
      const candidates: Array<Box & { anchor: "start" | "end" | "middle"; tx: number; ty: number }> = [
        { x: point.x + MARKER + 4, y: point.y - LABEL_LINE / 2, w, h: LABEL_LINE, anchor: "start", tx: point.x + MARKER + 4, ty: point.y + 5 },
        { x: point.x - MARKER - 4 - w, y: point.y - LABEL_LINE / 2, w, h: LABEL_LINE, anchor: "end", tx: point.x - MARKER - 4, ty: point.y + 5 },
        { x: point.x - w / 2, y: point.y - MARKER - 4 - LABEL_LINE, w, h: LABEL_LINE, anchor: "middle", tx: point.x, ty: point.y - MARKER - 8 },
        { x: point.x - w / 2, y: point.y + MARKER + 4, w, h: LABEL_LINE, anchor: "middle", tx: point.x, ty: point.y + MARKER + 17 },
      ];
      const inside = (box: Box) => box.x >= 2 && box.y >= 2 && box.x + box.w <= width() - 2 && box.y + box.h <= height() - 2;
      const spot = candidates.find((box) => inside(box) && !taken.some((other) => overlaps(box, other)));
      if (spot) taken.push(spot);
      return spot;
    });
  });

  const scaleBar = createMemo(() => {
    const metres = roundDown(view().metresPerPixel * Math.min(140, width() / 3));
    return { metres, pixels: metres / view().metresPerPixel };
  });

  const legs = createMemo(() => places().slice(1).map((place, index) => distance(places()[index], place)));
  const total = () => legs().reduce((sum, leg) => sum + leg, 0);

  const path = () => points().map((point, index) => `${index ? "L" : "M"}${point.x.toFixed(1)},${point.y.toFixed(1)}`).join(" ")
    + (connect() === "area" ? " Z" : "");

  // Street tiles, only while the owner allows them.
  const [loadedPlaces, setLoadedPlaces] = createSignal<Place[]>();
  const tiles = createMemo(() => {
    if (!mapSettings()?.tiles || loadedPlaces() !== places()) return [];
    const zoom = Math.max(0, Math.floor(Math.log2(view().scale / TILE)));
    const count = 2 ** zoom;
    const side = view().scale / count;
    const left = view().centreX - width() / 2 / view().scale;
    const top = view().centreY - height() / 2 / view().scale;
    const out: Array<{ key: string; z: number; x: number; y: number; px: number; py: number; side: number }> = [];
    for (let ty = Math.floor(top * count); ty < Math.ceil((top + height() / view().scale) * count); ty++) {
      if (ty < 0 || ty >= count) continue;
      for (let tx = Math.floor(left * count); tx < Math.ceil((left + width() / view().scale) * count); tx++) {
        const x = ((tx % count) + count) % count;
        out.push({ key: `${zoom}/${x}/${ty}`, z: zoom, x, y: ty, px: view().toX(tx / count), py: view().toY(ty / count), side });
      }
    }
    return out;
  });
  const [images, setImages] = createSignal<Record<string, string>>({});
  const asked = new Set<string>();
  createEffect(() => {
    for (const tile of tiles()) {
      if (asked.has(tile.key)) continue;
      asked.add(tile.key);
      api.mapTile(tile.z, tile.x, tile.y)
        .then((url) => setImages((current) => ({ ...current, [tile.key]: url })))
        .catch(() => undefined);
    }
  });
  onCleanup(() => Object.values(images()).forEach((url) => URL.revokeObjectURL(url)));

  const description = () => {
    const names = places().map((place) => place.label).join(", ");
    if (connect() === "route") return `Map of a route through ${names}`;
    if (connect() === "area") return `Map of an area with corners ${names}`;
    return `Map of ${names}`;
  };

  return (
    <Card form="map" label={p().label} title={p().title}>
      <Show when={mapSettings()?.tiles && loadedPlaces() !== places()}>
        <button type="button" class="btn" onClick={() => setLoadedPlaces(places())}>Load street map from OpenStreetMap</button>
      </Show>
      <div class="map-frame" ref={host}>
        <svg width={width()} height={height()} role="img" aria-label={description()}>
          <rect class="map-ground" x="0" y="0" width={width()} height={height()} />
          <Show when={tiles().length > 0}>
            <g class="map-tiles">
              <For each={tiles()}>{(tile) => (
                <Show when={images()[tile.key]}>{(href) => (
                  <image href={href()} x={tile.px} y={tile.py} width={tile.side + 0.5} height={tile.side + 0.5} preserveAspectRatio="none" />
                )}</Show>
              )}</For>
            </g>
          </Show>
          <Show when={connect() !== "none" && points().length > 1}>
            <path class={`map-${connect()}`} d={path()} />
          </Show>
          <For each={points()}>{(point, index) => (
            <g class="map-marker">
              <circle cx={point.x} cy={point.y} r={MARKER} />
              <text x={point.x} y={point.y + 4} text-anchor="middle">{index() + 1}</text>
              <title>{places()[index()].label}</title>
            </g>
          )}</For>
          <For each={labels()}>{(spot, index) => (
            <Show when={spot}>{(box) => (
              <text class="map-label" x={box().tx} y={box().ty} text-anchor={box().anchor}>{places()[index()].label}</text>
            )}</Show>
          )}</For>
          <g class="map-north" transform={`translate(${width() - 22}, 24)`}>
            <path d="M0,-12 L6,4 L0,0 L-6,4 Z" />
            <text y="18" text-anchor="middle">N</text>
          </g>
          <g class="map-scale" transform={`translate(14, ${height() - 16})`}>
            <path d={`M0,-6 V0 H${scaleBar().pixels.toFixed(1)} V-6`} />
            <text x="0" y="-10">{length(scaleBar().metres)}</text>
          </g>
          <Show when={tiles().length > 0}>
            <text class="map-attribution" x={width() - 8} y={height() - 8} text-anchor="end">{mapSettings()?.attribution}</text>
          </Show>
        </svg>
      </div>
      <ol class="map-legend">
        <For each={places()}>{(place, index) => (
          <li>
            <span class="map-badge" aria-hidden="true">{index() + 1}</span>
            <span class="map-place">
              <strong>{place.label}</strong>
              <Show when={place.detail}>{(detail) => <span>{detail()}</span>}</Show>
              <Show when={connect() === "route" && index() > 0}>
                <span class="map-leg">{length(legs()[index() - 1])} from {index()}</span>
              </Show>
            </span>
          </li>
        )}</For>
      </ol>
      <p class="card-meta">
        {connect() === "route" && places().length > 1 ? `${length(total())} in all, in straight lines. ` : ""}
        {connect() === "area" && places().length > 2 ? `About ${size(area(places()))} inside. ` : ""}
        Positions as given.
      </p>
    </Card>
  );
}
