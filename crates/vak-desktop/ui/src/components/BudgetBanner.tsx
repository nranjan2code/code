import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import { setPendingSettingsPage, setSettingsOpen } from "../store";
import * as api from "../api";
import Icon from "./Icon";

const POLL_MS = 20_000;

/**
 * Proactive budget alert (docs/design/29-personal-os.md): amber at ≥80% of
 * the day cap, red at ≥100%. Dismissal lasts for the app session AND the
 * calendar day — a new day re-arms the banner because yesterday's dismissal
 * says nothing about today's spend.
 */
export default function BudgetBanner() {
  const [ratio, setRatio] = createSignal<number | null>(null);
  const [spent, setSpent] = createSignal(0);
  const [cap, setCap] = createSignal(0);
  const [dismissedDay, setDismissedDay] = createSignal<string | null>(null);

  const todayKey = () => new Date().toISOString().slice(0, 10);

  const refresh = async () => {
    try {
      const f = await api.finopsStatus();
      if (f.day_cap_usd == null || f.day_cap_usd <= 0) {
        setRatio(null);
        return;
      }
      setSpent(f.day_usd);
      setCap(f.day_cap_usd);
      setRatio(f.day_usd / f.day_cap_usd);
      // A stale dismissal from an earlier day must not mute today.
      if (sessionStorage.getItem("vak.budgetDismissed") === todayKey()) {
        setDismissedDay(todayKey());
      } else {
        setDismissedDay(null);
        sessionStorage.removeItem("vak.budgetDismissed");
      }
    } catch {
      /* backend down — banner simply stays quiet */
    }
  };

  createEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), POLL_MS);
    onCleanup(() => clearInterval(t));
  });

  const level = (): "amber" | "red" | null => {
    const r = ratio();
    if (r == null) return null;
    if (r >= 1) return "red";
    if (r >= 0.8) return "amber";
    return null;
  };

  createEffect(() => {
    if (level() === "red" && document.hidden) {
      void import("../App").then((m) =>
        m.notifyOnce(
          "budget-red",
          "Vak day budget exceeded",
          `$${spent().toFixed(2)} of $${cap().toFixed(2)} — new runs may be denied.`,
        ),
      );
    }
  });

  const dismiss = () => {
    sessionStorage.setItem("vak.budgetDismissed", todayKey());
    setDismissedDay(todayKey());
  };

  const openBudget = () => {
    setPendingSettingsPage("services");
    setSettingsOpen(true);
  };

  return (
    <Show when={level() && dismissedDay() === null}>
      <div class="budget-banner" classList={{ red: level() === "red" }} role="alert">
        <span class="budget-banner-dot" aria-hidden="true" />
        <strong>{level() === "red" ? "Day budget exceeded" : "Approaching day budget"}</strong>
        <span>
          ${spent().toFixed(2)} of ${cap().toFixed(2)} ({Math.round((ratio() ?? 0) * 100)}%) — new runs may be denied until tomorrow.
        </span>
        <button class="settings-button" onClick={openBudget}>View budget</button>
        <button class="icon-button subtle has-tooltip" data-tooltip="Dismiss for today" aria-label="Dismiss budget alert for today" onClick={dismiss}>
          <Icon name="close" size={13} />
        </button>
      </div>
    </Show>
  );
}
