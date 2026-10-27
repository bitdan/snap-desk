import type { PinSummary } from '@/features/pins/types'

export function pinGroups(pins: PinSummary[]): string[] {
  return [...new Set(pins.map(pin => pin.group).filter(Boolean))].sort((a, b) => a.localeCompare(b))
}

/** null 是全部，空字符串是未分组，两者不可混淆为同一个批量操作范围。 */
export function filterPins(pins: PinSummary[], group: string | null): PinSummary[] {
  return group === null ? pins : pins.filter(pin => pin.group === group)
}

export function normalizePinGroup(value: string): string {
  const group = value.trim()
  if ([...group].length > 32 || /[\u0000-\u001f\u007f-\u009f]/u.test(group)) throw new Error('分组名称最多 32 个字符，且不能包含控制字符。')
  return group
}
