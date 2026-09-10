import { For, Show, createSignal } from "solid-js";
import { safeUrl } from "../../safeUrl";

export interface ResearchSource {
  title: string;
  url: string;
  snippet?: string;
  source_name?: string;
  published_at?: string;
}

export interface ResearchTakeaway {
  text: string;
  citation_indices?: number[];
}

export interface ResearchData {
  title?: string;
  takeaways: (string | ResearchTakeaway)[];
  sources: ResearchSource[];
}

export default function ResearchCards(props: { data: ResearchData }) {
  const [copied, setCopied] = createSignal(false);

  const handleCopy = async () => {
    const text = props.data.takeaways
      .map((t) => (typeof t === "string" ? t : t.text))
      .join("\n• ");
    try {
      await navigator.clipboard.writeText(`• ${text}`);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div class="canvas-card research-card-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">Research Synthesis</span>
          <span class="card-subtitle">{props.data.sources.length} sources</span>
        </div>
        <div class="card-actions">
          <button type="button" class="pill-action-btn" onClick={handleCopy}>
            {copied() ? "✓ Copied" : "Copy Synthesis"}
          </button>
        </div>
      </div>
      
      <div class="research-grid">
        <div class="takeaways-list">
          <For each={props.data.takeaways}>
            {(item, idx) => {
              const text = typeof item === "string" ? item : item.text;
              const cites = typeof item === "object" ? item.citation_indices ?? [] : [];
              return (
                <div class="takeaway-row">
                  <div class="takeaway-bullet">{idx() + 1}</div>
                  <div class="takeaway-content">
                    <span>{text}</span>
                    <For each={cites}>
                      {(citeIdx) => {
                        const source = props.data.sources[citeIdx - 1];
                        return (
                          <span class="inline-cite">
                            [{citeIdx}]
                            <Show when={source}>
                              <div class="cite-popover">
                                <div class="cite-source-badge">{source?.source_name ?? "Source"}</div>
                                <strong>{source?.title}</strong>
                                <Show when={source?.snippet}>
                                  <p class="cite-snippet">"{source?.snippet}"</p>
                                </Show>
                              </div>
                            </Show>
                          </span>
                        );
                      }}
                    </For>
                  </div>
                </div>
              );
            }}
          </For>
        </div>

        <Show when={props.data.sources.length > 0}>
          <div class="source-shelf">
            <For each={props.data.sources}>
              {(src) => (
                <a
                  class="source-tile"
                  href={safeUrl(src.url) ? src.url : undefined}
                  target="_blank"
                  rel="noreferrer noopener"
                >
                  <div class="source-favicon">🌐</div>
                  <div class="source-info">
                    <strong>{src.title}</strong>
                    <small>{src.source_name ?? src.url}</small>
                  </div>
                </a>
              )}
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
}
