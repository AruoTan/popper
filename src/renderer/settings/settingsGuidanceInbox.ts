import type { SettingsGuidance } from '../../shared'

export interface SettingsGuidanceLease {
  id: number
  value: SettingsGuidance
}

export interface SettingsGuidanceInbox {
  acquire(take: () => Promise<SettingsGuidance | null>): Promise<SettingsGuidanceLease | null>
  push(value: SettingsGuidance): SettingsGuidanceLease
  acknowledge(id: number): void
}

function hasGuidance(value: SettingsGuidance): boolean {
  return Boolean(value.focus?.trim() || value.notice?.trim())
}

export function createSettingsGuidanceInbox(): SettingsGuidanceInbox {
  const queue: SettingsGuidanceLease[] = []
  let inFlight: Promise<SettingsGuidanceLease | null> | null = null
  let nextId = 1

  const enqueue = (value: SettingsGuidance): SettingsGuidanceLease => {
    const lease = { id: nextId, value }
    nextId += 1
    queue.push(lease)
    return lease
  }

  return {
    acquire(take): Promise<SettingsGuidanceLease | null> {
      if (queue[0]) return Promise.resolve(queue[0])
      if (inFlight) return inFlight

      const attempt = take().then((value) => {
        if (!value || !hasGuidance(value)) return null
        return enqueue(value)
      })
      inFlight = attempt
      const clearInFlight = (): void => {
        if (inFlight === attempt) inFlight = null
      }
      void attempt.then(clearInFlight, clearInFlight)
      return attempt
    },
    push(value): SettingsGuidanceLease {
      return enqueue(value)
    },
    acknowledge(id): void {
      if (queue[0]?.id === id) queue.shift()
    }
  }
}

export const settingsGuidanceInbox = createSettingsGuidanceInbox()
