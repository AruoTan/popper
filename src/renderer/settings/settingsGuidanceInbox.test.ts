import type { SettingsGuidance } from '../../shared'
import { createSettingsGuidanceInbox } from './settingsGuidanceInbox'

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise
  })
  return { promise, resolve }
}

describe('SettingsGuidanceInbox', () => {
  it('shares an in-flight acquire and retains the lease until acknowledgement', async () => {
    const inbox = createSettingsGuidanceInbox()
    const pending = deferred<SettingsGuidance | null>()
    const take = vi.fn(() => pending.promise)
    const firstAcquire = inbox.acquire(take)
    const remountAcquire = inbox.acquire(take)

    expect(take).toHaveBeenCalledTimes(1)

    pending.resolve({ focus: 'providers', notice: '请配置服务商' })
    const firstLease = await firstAcquire
    expect(await remountAcquire).toEqual(firstLease)
    expect(await inbox.acquire(take)).toEqual(firstLease)

    inbox.acknowledge(firstLease!.id)
    expect(await inbox.acquire(vi.fn().mockResolvedValue(null))).toBeNull()
  })

  it('queues native events behind an unacknowledged lease in FIFO order', async () => {
    const inbox = createSettingsGuidanceInbox()
    const first = inbox.push({ focus: 'providers', notice: 'first' })
    const second = inbox.push({ focus: 'actions', notice: 'second' })

    expect(await inbox.acquire(vi.fn())).toEqual(first)
    inbox.acknowledge(second.id)
    expect(await inbox.acquire(vi.fn())).toEqual(first)

    inbox.acknowledge(first.id)
    expect(await inbox.acquire(vi.fn())).toEqual(second)
    inbox.acknowledge(second.id)
    expect(await inbox.acquire(vi.fn().mockResolvedValue(null))).toBeNull()
  })

  it('does not enqueue empty guidance returned by the native take', async () => {
    const inbox = createSettingsGuidanceInbox()
    expect(await inbox.acquire(vi.fn().mockResolvedValue({}))).toBeNull()
  })
})
