import { For, Show, createMemo, createSignal, useContext } from "solid-js";
import { daysOf, evaluateExpression, parseExpression } from "../../controlsExpression";
import { Card, PresentationInteractionContext, num, records, text, type Payload } from "./shared";

let nextControl = 0;

/** What an input holds while the person changes it: its text as typed, so a
 *  half-typed number is not thrown away. */
type Setting = Record<string, string>;

function initial(input: Payload): string {
  if (typeof input.value === "boolean") return input.value ? "true" : "false";
  return text(input.value) ?? "";
}

function valueOf(input: Payload, written: string): number | undefined {
  switch (text(input.kind)) {
    case "toggle": return written === "true" ? 1 : 0;
    case "date": return daysOf(written);
    default: {
      const number = written.trim() === "" ? NaN : Number(written);
      return Number.isFinite(number) ? number : undefined;
    }
  }
}

/** Inputs a person changes and the results they give, computed here as they
 *  change (docs/design/96, P1). The card's own values are where it starts;
 *  "Ask about this" sends what the person set, and the host computes the
 *  results again before the turn starts. */
export default function ControlsCard(props: { payload: Payload }) {
  const p = () => props.payload;
  const interaction = useContext(PresentationInteractionContext);
  const inputs = () => records(p().inputs);
  const outputs = () => records(p().outputs);
  const [setting, setSetting] = createSignal<Setting>(
    Object.fromEntries(records(p().inputs).map((input) => [text(input.name) ?? "", initial(input)])),
  );
  const set = (name: string, written: string) => setSetting({ ...setting(), [name]: written });
  const results = createMemo(() => {
    const values: Record<string, number> = {};
    for (const input of inputs()) {
      const value = valueOf(input, setting()[text(input.name) ?? ""] ?? "");
      if (value === undefined) return { error: `${text(input.label) ?? text(input.name)} needs a value` };
      values[text(input.name) ?? ""] = value;
    }
    const shown: { label: string; value: string; unit?: string }[] = [];
    for (const output of outputs()) {
      try {
        const value = evaluateExpression(parseExpression(text(output.expression) ?? ""), values);
        values[text(output.name) ?? ""] = value;
        const decimals = Math.min(num(output.decimals) ?? 0, 12);
        shown.push({
          label: text(output.label) ?? "",
          value: value.toLocaleString(undefined, { minimumFractionDigits: decimals, maximumFractionDigits: decimals }),
          unit: text(output.unit),
        });
      } catch (error) {
        return { error: `${text(output.label) ?? ""}: ${error instanceof Error ? error.message : String(error)}` };
      }
    }
    return { shown };
  });
  const changed = () => inputs().some((input) => (setting()[text(input.name) ?? ""] ?? "") !== initial(input));
  const ask = () => {
    if (!interaction?.onValues) return;
    const values = Object.fromEntries(inputs()
      .map((input) => [text(input.name) ?? "", setting()[text(input.name) ?? ""] ?? ""] as const)
      .filter(([name, written]) => written !== initial(inputs().find((input) => text(input.name) === name) ?? {})));
    interaction.onValues(values);
  };
  return (
    <Card form="controls" label={p().label} title={p().title}>
      <div class="controls">
        <div class="controls-inputs">
          <For each={inputs()}>{(input) => {
            const name = text(input.name) ?? "";
            const id = `control-${(nextControl += 1)}`;
            const unit = () => text(input.unit);
            const written = () => setting()[name] ?? "";
            const kind = text(input.kind);
            return (
              <div class="control" data-kind={kind}>
                <Show when={kind === "toggle" || kind === "choice"} fallback={<label class="control-label" for={id}>{text(input.label)}</label>}>
                  <span class="control-label" id={`${id}-label`}>{text(input.label)}</span>
                </Show>
                <div class="control-field">
                  <Show when={kind === "choice"}>
                    <div class="control-choices" role="radiogroup" aria-labelledby={`${id}-label`}>
                      <For each={records(input.options)}>{(option) => (
                        <button type="button" role="radio" class="control-choice"
                          aria-checked={written() === text(option.value)}
                          onClick={() => set(name, text(option.value) ?? "")}>{text(option.label)}</button>
                      )}</For>
                    </div>
                  </Show>
                  <Show when={kind === "toggle"}>
                    <input id={id} type="checkbox" role="switch" class="control-switch" aria-labelledby={`${id}-label`}
                      checked={written() === "true"} onChange={(event) => set(name, event.currentTarget.checked ? "true" : "false")} />
                  </Show>
                  <Show when={kind === "date"}>
                    <input id={id} type="date" class="control-input" value={written()} onInput={(event) => set(name, event.currentTarget.value)} />
                  </Show>
                  <Show when={kind === "range"}>
                    <input id={id} type="range" class="control-range" min={num(input.min)} max={num(input.max)} step={num(input.step) ?? "any"}
                      value={written()} onInput={(event) => set(name, event.currentTarget.value)} />
                    <output class="control-reading" for={id}>{written()}<Show when={unit()}>{(u) => <span class="control-unit">{u()}</span>}</Show></output>
                  </Show>
                  <Show when={kind === "number"}>
                    <input id={id} type="number" inputmode="decimal" class="control-input" min={num(input.min)} max={num(input.max)} step={num(input.step) ?? "any"}
                      value={written()} onInput={(event) => set(name, event.currentTarget.value)} />
                    <Show when={unit()}>{(u) => <span class="control-unit">{u()}</span>}</Show>
                  </Show>
                </div>
              </div>
            );
          }}</For>
        </div>
        <Show when={results().shown} fallback={<p class="controls-error" role="status">{results().error}</p>}>
          {(shown) => (
            <div class="stats controls-outputs" aria-live="polite">
              <For each={shown()}>{(result) => (
                <div class="stat">
                  <div class="stat-value">{result.value}<Show when={result.unit}>{(u) => <span class="stat-unit">{u()}</span>}</Show></div>
                  <div class="stat-label">{result.label}</div>
                </div>
              )}</For>
            </div>
          )}
        </Show>
        <Show when={interaction?.onValues}>
          <div class="controls-actions">
            <button type="button" class="card-button" disabled={!changed() || !results().shown} onClick={ask}>Ask about this</button>
          </div>
        </Show>
      </div>
    </Card>
  );
}
