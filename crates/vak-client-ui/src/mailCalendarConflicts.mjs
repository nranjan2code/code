/**
 * Find overlapping calendar entries by provider ID. All-day date bounds are
 * local calendar days with an exclusive end; timed bounds are instants.
 * Event text and privacy flags are intentionally irrelevant to conflict math.
 * @param {Array<{provider_id:string,all_day:boolean,starts_on:string|null,ends_on:string|null,starts_at:string|null,ends_at:string|null}>} events
 * @returns {Set<string>}
 */
export function overlappingMailCalendarEventIds(events) {
  const localDayStart = (value) => {
    if (!value || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
    const [year, month, day] = value.split("-").map(Number);
    const date = new Date(year, month - 1, day);
    if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day) return null;
    return date.getTime();
  };
  const intervals = events.flatMap((event) => {
    const start = event.all_day ? localDayStart(event.starts_on) : event.starts_at ? Date.parse(event.starts_at) : NaN;
    const end = event.all_day ? localDayStart(event.ends_on) : event.ends_at ? Date.parse(event.ends_at) : NaN;
    return Number.isFinite(start) && Number.isFinite(end) && end > start
      ? [{ id: event.account_id ? `${event.account_id}::${event.provider_id}` : event.provider_id, start, end }]
      : [];
  }).sort((left, right) => left.start - right.start || left.end - right.end);
  const conflicts = new Set();
  for (let index = 0; index < intervals.length; index += 1) {
    const current = intervals[index];
    for (let next = index + 1; next < intervals.length && intervals[next].start < current.end; next += 1) {
      if (current.id !== intervals[next].id && intervals[next].end > current.start) {
        conflicts.add(current.id);
        conflicts.add(intervals[next].id);
      }
    }
  }
  return conflicts;
}
