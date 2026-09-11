import { For, Show, createSignal, onCleanup } from "solid-js";

export interface Ingredient {
  name: string;
  amount?: number;
  unit?: string;
}

export interface CookingStep {
  text: string;
  timer_seconds?: number;
}

export interface RecipeData {
  title: string;
  prep_time_minutes?: number;
  cook_time_minutes?: number;
  servings?: number;
  ingredients: (string | Ingredient)[];
  steps: (string | CookingStep)[];
}

export default function RecipeCard(props: { data: RecipeData }) {
  const [servings, setServings] = createSignal(props.data.servings ?? 1);
  const baseServings = props.data.servings ?? 1;

  const [activeTimers, setActiveTimers] = createSignal<Record<number, number>>({});
  const [timerIntervals, setTimerIntervals] = createSignal<Record<number, ReturnType<typeof setInterval>>>({});

  const scale = () => servings() / baseServings;

  const toggleTimer = (idx: number, totalSeconds: number) => {
    const intervals = timerIntervals();
    if (intervals[idx]) {
      clearInterval(intervals[idx]);
      const newIntervals = { ...intervals };
      delete newIntervals[idx];
      setTimerIntervals(newIntervals);

      const newTimers = { ...activeTimers() };
      delete newTimers[idx];
      setActiveTimers(newTimers);
      return;
    }

    const newTimers = { ...activeTimers(), [idx]: totalSeconds };
    setActiveTimers(newTimers);

    const deadline = Date.now() + totalSeconds * 1000;
    const interval = setInterval(() => {
      setActiveTimers((prev) => {
        const current = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
        if (current === 0) {
          clearInterval(interval);
          const nextIntervals = { ...timerIntervals() };
          delete nextIntervals[idx];
          setTimerIntervals(nextIntervals);

          return { ...prev, [idx]: 0 };
        }
        return { ...prev, [idx]: current };
      });
    }, 1000);

    setTimerIntervals({ ...intervals, [idx]: interval });
  };

  onCleanup(() => {
    for (const id of Object.values(timerIntervals())) {
      clearInterval(id);
    }
  });

  const formatTimer = (seconds: number) => {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return seconds === 0 ? "Timer complete · Restart" : `${m}:${s < 10 ? "0" : ""}${s} · Stop`;
  };

  return (
    <div class="canvas-card culinary-card-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-amber">Culinary Recipe</span>
          <span class="card-subtitle">
            {props.data.title}<Show when={props.data.cook_time_minutes !== undefined}> · {props.data.cook_time_minutes} min</Show>
          </span>
        </div>
        <div class="card-actions">
          <Show when={props.data.servings !== undefined}>
          <span style={{ "font-size": "12px", color: "var(--text-muted)" }}>Servings:</span>
          <div class="servings-stepper">
            <button type="button" aria-label="Decrease servings" disabled={servings() <= 1} onClick={() => setServings(Math.max(1, servings() - 1))}>-</button>
            <span class="servings-num">{servings()}</span>
            <button type="button" aria-label="Increase servings" disabled={servings() >= 10000} onClick={() => setServings(servings() + 1)}>+</button>
          </div>
          </Show>
        </div>
      </div>

      <div class="culinary-stage">
        <div class="ingredients-column">
          <span class="column-subhead">Ingredients</span>
          <For each={props.data?.ingredients || []}>
            {(item) => {
              if (typeof item === "string") {
                return (
                  <label class="ingredient-checkbox-row">
                    <input type="checkbox" />
                    <span>{item}</span>
                  </label>
                );
              }
              const name = () => item.name || (item as any).item || (item as any).ingredient || "";
              const scaledAmount = () => item.amount === undefined ? null : Math.round(item.amount * scale() * 100) / 100;
              return (
                <label class="ingredient-checkbox-row">
                  <input type="checkbox" />
                  <span>
                    {scaledAmount() === null ? "" : `${scaledAmount()} `}
                    {item.unit ? `${item.unit} ` : ""}
                    {name()}
                  </span>
                </label>
              );
            }}
          </For>
        </div>

        <div class="directions-column">
          <span class="column-subhead">Directions & Timers</span>
          <For each={props.data?.steps || []}>
            {(step, idx) => {
              const text = typeof step === "string" ? step : (step.text || (step as any).step || (step as any).instruction || "");
              const timerSecs = typeof step === "object" ? step.timer_seconds : null;
              const isRunning = () => activeTimers()[idx()] !== undefined;
              return (
                <div class="timer-action-card" classList={{ "active-timer": isRunning() }}>
                  <div>
                    <div style={{ "font-size": "13px", "font-weight": "600", color: "var(--text-main)" }}>
                      {idx() + 1}. {text}
                    </div>
                  </div>
                  {timerSecs && (
                    <button
                      class="timer-trigger-btn"
                      classList={{ running: isRunning() }}
                      onClick={() => toggleTimer(idx(), timerSecs)}
                    >
                      {isRunning()
                        ? formatTimer(activeTimers()[idx()])
                        : `Start ${Math.floor(timerSecs / 60)}:${String(timerSecs % 60).padStart(2, "0")} timer`}
                    </button>
                  )}
                </div>
              );
            }}
          </For>
        </div>
      </div>
    </div>
  );
}
