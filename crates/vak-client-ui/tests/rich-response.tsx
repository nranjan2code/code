import { render } from "solid-js/web";
import "../src/components/ChatPane";
import PageView from "../src/components/PageView";
import * as store from "../src/store";
import type { OutputTimeline } from "../src/types";
import corpus from "../../vak-delivery/tests/fixtures/rich-answer.json";
import "../src/styles.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";
const timeline = structuredClone(corpus) as unknown as OutputTimeline;
timeline.items.push({ ...timeline.items[2], id:"shared-file", kind:"artifact", content:{type:"artifact",artifact:{name:"review.docx",path:"review.docx",status:{state:"in_folder"}}} });
const longUrl = "https://api.open-meteo.com/v1/forecast?latitude=28.58&longitude=77.33&current=temperature_2m,relative_humidity_2m,apparent_temperature,is_day,precipitation,weather_code,wind_speed_10m,wind_direction_10m&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset&timezone=Asia%2FKolkata&forecast_days=1";
const sourceItem = timeline.items.find(item => item.id === "sources-entry")!;
if (sourceItem.content.type === "structured") {
  const payload = sourceItem.content.output.payload as { sources: Record<string, unknown>[] };
  payload.sources.push(
    { title: longUrl, url: longUrl, source_name: longUrl, finding: "Retrieved in this turn." },
    { title: "Weather conditions and forecast", url: longUrl + "&named=1", source_name: "Open-Meteo", finding: "Named sources keep their titles." },
    { title: "Long source title " + "unbroken".repeat(160), url: "https://example.com/long", finding: "unbroken".repeat(320) },
    { title: "Unsafe source", url: "javascript:alert(1)", finding: "Unsafe destinations stay inert." },
  );
}
const requests: string[] = [];
window.fetch = async (input) => { requests.push(String(input)); return new Response("{}", {headers:{"Content-Type":"application/json"}}); };
store.setTechnicalDetails(false);
render(() => <main style="max-width:820px;margin:0 auto;padding:16px"><PageView page={{format:"vak-page",title:"Rich response",items:timeline.items,text:""}} versionId="fixture" hideUser={false} /></main>, document.getElementById("root")!);
(window as any).runChecks = () => {
  const body = document.querySelector("main")!;
  const text = body.textContent ?? "";
  const order = ["Leading context survives.","Observed series","Note: these are supplied observations", "0007", "12345678901234567890.001", "Original observations"];
  if (!order.every((part,index) => text.includes(part) && (!index || text.indexOf(part) > text.indexOf(order[index-1])))) throw new Error("Rich response order or exact scalars changed");
  const chart = body.querySelector('svg[role="img"][aria-label="R3 to R4 changes by fourteen."]');
  const points = Array.from(chart?.querySelectorAll("circle title") ?? []).map(point => point.textContent);
  if (JSON.stringify(points) !== JSON.stringify(["Readings: R1, 10", "Readings: R2, 12", "Readings: R3, 11", "Readings: R4, 25"])) throw new Error("Rich chart lost its supplied observations");
  if (!text.includes("Partial result") || !text.includes("Check the evidence")) throw new Error("Result verdict missing");
  if (document.documentElement.scrollWidth > innerWidth || body.scrollWidth > body.clientWidth) throw new Error("Answer overflows viewport");
  for (const element of body.querySelectorAll<HTMLElement>(".source, .card, .primary-result, .assistant-turn-body")) {
    if (element.scrollWidth > element.clientWidth + 1) throw new Error("Answer content escaped " + element.className);
  }
  const sourceLinks = Array.from(body.querySelectorAll<HTMLAnchorElement>(".source-from a"));
  const raw = sourceLinks.find(link => link.getAttribute("href") === longUrl);
  if (raw?.textContent !== "api.open-meteo.com") throw new Error("Raw URL did not become a short source link");
  if (!sourceLinks.some(link => link.textContent === "Weather conditions and forecast" && link.getAttribute("href") === longUrl + "&named=1")) throw new Error("Named source or complete destination changed");
  if (body.querySelector('.source-from a[href^="javascript:"]')) throw new Error("Unsafe source became active");
  if (body.querySelector('button[aria-label="Edit prompt in composer"]')) throw new Error("Shared view offers owner composer edit");
  if (body.querySelector('button[aria-label="Review changes"]')) throw new Error("Saved/shared view offers owner review mutation");
  if (requests.length) throw new Error("Saved/shared page called an owner API: " + requests.join(","));
  return ["No ambient owner API calls for shared files", "Shared page renderer preserves prose/card order and exact scalars", "Rich chart and result verdict remain visible", "Saved/shared view offers no owner mutation", "Answer fits viewport"];
};

if (new URLSearchParams(location.search).has("autocheck")) {
  void document.fonts.ready.then(() => requestAnimationFrame(() => requestAnimationFrame(() => {
    let result: string;
    try { (window as any).runChecks(); result = "PASS"; } catch (error) { result = "FAIL: " + String(error); }
    const status = document.createElement("p"); status.textContent = result; status.setAttribute("role", "status"); document.body.prepend(status);
    parent.postMessage({ fixture: "rich-response", result }, location.origin);
  })));
}
