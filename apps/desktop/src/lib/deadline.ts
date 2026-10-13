import { Temporal } from '@js-temporal/polyfill';
import type { Task } from './types';

/** Never cache this value: the system zone can change while a window stays open. */
export function currentTimeZone(): string {
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  // Android WebView can report a numeric zero offset for the system UTC zone.
  return zone === '+00:00' || zone === '-00:00' ? 'UTC' : zone;
}
export function validDate(value: unknown): value is string {
  if (typeof value !== 'string' || !/^(?!0000)\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  try { return Temporal.PlainDate.from(value).toString() === value; } catch { return false; }
}
export function validTime(value: unknown): value is string {
  return typeof value === 'string' && /^([01]\d|2[0-3]):[0-5]\d$/.test(value);
}
export function validTimeZone(value: unknown): value is string {
  // Temporal also accepts numeric offsets. The persisted contract requires an IANA name.
  if (typeof value !== 'string' || !value || /^[+-]/.test(value)) return false;
  try { Temporal.Instant.fromEpochMilliseconds(0).toZonedDateTimeISO(value); return true; } catch { return false; }
}
export function validUtcInstant(value: unknown): value is string {
  if (typeof value !== 'string' || !/^(?!0000)\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:[0-5]\d(?:\.\d{1,9})?(?:Z|\+00:00)$/.test(value)) return false;
  try { Temporal.Instant.from(value); return true; } catch { return false; }
}
export function localDate(date = new Date()): string {
  return `${String(date.getFullYear()).padStart(4, '0')}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}
export function dateOffset(date: string, days: number): string {
  if (!validDate(date) || !Number.isSafeInteger(days)) throw new Error('日期格式无效');
  const result = Temporal.PlainDate.from(date).add({ days }).toString();
  if (!validDate(result)) throw new Error('日期超出支持范围');
  return result;
}
export function resolveDeadline(date: string, time: string | null, zone: string): string | null {
  if (!validDate(date)) throw new Error('请选择有效的截止日期');
  if (time !== null && !validTime(time)) throw new Error('请选择有效的截止时间');
  if (!validTimeZone(zone)) throw new Error('无法识别截止日期的时区，请重新选择日期');
  const day = Temporal.PlainDate.from(date);
  if (day.toZonedDateTime(zone).toPlainDate().toString() !== date) throw new Error('所选日期在该时区不存在，请选择其他日期');
  if (time === null) return null;
  try {
    return day.toPlainDateTime(time).toZonedDateTime(zone, { disambiguation: 'reject' }).toInstant().toString();
  } catch {
    throw new Error('该时刻因夏令时不存在或出现两次，请选择其他时刻');
  }
}
/** Validate stored data without recalculating a saved instant after timezone-rule updates. */
export function validateDeadline(task: Task): void {
  if (task.dueDate !== null && !validDate(task.dueDate)) throw new Error('请选择有效的截止日期');
  if (task.dueTime !== null && (!task.dueDate || !validTime(task.dueTime))) throw new Error('请选择有效的截止时间');
  const zone = task.dueTimezone;
  const instant = task.dueAtUtc;
  if (!task.dueDate) {
    if (zone != null || instant != null) throw new Error('无截止日期时不能保存时区或截止瞬间');
  } else if (zone != null) {
    if (!validTimeZone(zone)) throw new Error('截止日期的时区无效');
    if (task.dueTime !== null) {
      if (!validUtcInstant(instant)) throw new Error('精确截止时间缺少有效的 UTC 瞬间');
    } else {
      if (instant != null) throw new Error('仅日期截止不能包含精确瞬间');
      resolveDeadline(task.dueDate, null, zone);
    }
  } else if (instant != null) {
    throw new Error('精确截止瞬间缺少来源时区');
  }
}
/** Apply only when the user actually changes a DDL; reading or editing notes never upgrades legacy data. */
export function fixDeadline(task: Task, previous?: Task): void {
  const changed = !previous || task.dueDate !== previous.dueDate || task.dueTime !== previous.dueTime || (task.dueTimezone ?? null) !== (previous.dueTimezone ?? null);
  if (!changed) return;
  if (!task.dueDate) {
    task.dueTimezone = null;
    task.dueAtUtc = null;
    return;
  }
  const zone = task.dueTimezone ?? previous?.dueTimezone ?? currentTimeZone();
  task.dueAtUtc = resolveDeadline(task.dueDate, task.dueTime, zone);
  task.dueTimezone = zone;
}
export function compareUtcInstants(a: string, b: string): number {
  return Temporal.Instant.compare(a, b);
}
/** A sortable key without losing the sub-millisecond precision accepted by imports. */
export function utcInstantNanoseconds(value: string): bigint {
  return Temporal.Instant.from(value).epochNanoseconds;
}
export function effectiveDeadline(task: Task, localZone?: string): number {
  if (!task.dueDate) return Infinity;
  if (task.dueAtUtc) {
    const at = Temporal.Instant.from(task.dueAtUtc);
    // Date.now has millisecond resolution; never report an imported sub-ms deadline early.
    return at.epochMilliseconds + (at.epochNanoseconds % 1_000_000n === 0n ? 0 : 1);
  }
  if (task.dueTimezone) return Temporal.PlainDate.from(task.dueDate).add({ days: 1 }).toZonedDateTime(task.dueTimezone).epochMilliseconds;
  // Legacy records deliberately retain the old floating-local interpretation until edited.
  if (task.dueTime) return new Date(`${task.dueDate}T${task.dueTime}:00`).getTime();
  return Temporal.PlainDate.from(task.dueDate).add({ days: 1 }).toZonedDateTime(localZone ?? currentTimeZone()).epochMilliseconds;
}
export function deadlineDisplay(task: Task, now = Date.now()): { date: string | null; time: string | null; today: string } {
  if (task.dueAtUtc) {
    const local = Temporal.Instant.from(task.dueAtUtc).toZonedDateTimeISO(currentTimeZone());
    return { date: local.toPlainDate().toString(), time: local.toPlainTime().toString({ smallestUnit: 'minute' }), today: localDate(new Date(now)) };
  }
  const today = task.dueTimezone
    ? Temporal.Instant.fromEpochMilliseconds(now).toZonedDateTimeISO(task.dueTimezone).toPlainDate().toString()
    : localDate(new Date(now));
  return { date: task.dueDate, time: task.dueTime, today };
}
export function deadlineTimeZoneHint(task: Task): string | null {
  if (!task.dueDate) return null;
  if (!task.dueTimezone) return '旧截止日期尚未固定时区；修改截止日期或时刻后会按当前时区保存。';
  return `截止日期时区：${task.dueTimezone}${task.dueAtUtc ? '；精确截止瞬间已固定' : ''}`;
}
/** Next view invalidation, including fixed-zone date labels. No persistent revision changes. */
export function nextTimeBoundary(tasks: readonly Task[], now = Date.now()): number {
  const zones = new Set([currentTimeZone(), ...tasks.flatMap(task => task.deletedAt == null && task.dueDate && !task.dueTime && task.dueTimezone ? [task.dueTimezone] : [])]);
  const boundaries = [...zones].map(zone => {
    const current = Temporal.Instant.fromEpochMilliseconds(now).toZonedDateTimeISO(zone);
    return current.toPlainDate().add({ days: 1 }).toZonedDateTime(zone).epochMilliseconds;
  });
  for (const task of tasks) {
    if (task.completed || task.deletedAt != null) continue;
    const at = effectiveDeadline(task);
    if (Number.isFinite(at) && at > now) boundaries.push(at);
  }
  return Math.min(...boundaries);
}
