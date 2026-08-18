/* eslint-env jest */
import { Supervisor, type ManagedProcess, type ProcessSpec } from './supervisor'

function recorder() {
  const events: string[] = []
  let nextPid = 100
  const spawn = (spec: ProcessSpec): ManagedProcess => {
    const pid = nextPid++
    events.push(`start ${spec.name} #${pid}`)
    return {
      pid,
      async stop() {
        events.push(`stop ${spec.name} #${pid}`)
      },
    }
  }
  return { events, spawn }
}

const spec = (name: string): ProcessSpec => ({
  name,
  command: 'true',
  args: [],
})

describe('Supervisor', () => {
  it('starts and tracks a process', async () => {
    const { events, spawn } = recorder()
    const supervisor = new Supervisor({ spawn })

    await supervisor.start(spec('server'))
    expect(supervisor.names()).toEqual(['server'])
    expect(supervisor.isRunning('server')).toBe(true)
    expect(events).toEqual(['start server #100'])
  })

  it('replaces a process started under the same name', async () => {
    const { events, spawn } = recorder()
    const supervisor = new Supervisor({ spawn })

    await supervisor.start(spec('server'))
    await supervisor.start(spec('server'))

    expect(events).toEqual([
      'start server #100',
      'stop server #100',
      'start server #101',
    ])
    expect(supervisor.names()).toEqual(['server'])
  })

  it('restarts a known process and reports an unknown one', async () => {
    const { events, spawn } = recorder()
    const supervisor = new Supervisor({ spawn })

    await supervisor.start(spec('renderer'))
    expect(await supervisor.restart('renderer')).toBe(true)
    expect(await supervisor.restart('ghost')).toBe(false)

    expect(events).toEqual([
      'start renderer #100',
      'stop renderer #100',
      'start renderer #101',
    ])
  })

  it('stops everything in reverse start order', async () => {
    const { events, spawn } = recorder()
    const supervisor = new Supervisor({ spawn })

    await supervisor.start(spec('server'))
    await supervisor.start(spec('renderer'))
    await supervisor.start(spec('next'))
    events.length = 0

    await supervisor.stopAll()
    expect(events).toEqual([
      'stop next #102',
      'stop renderer #101',
      'stop server #100',
    ])
    expect(supervisor.names()).toEqual([])
  })

  it('serialises overlapping restarts so stop/start never interleave', async () => {
    const events: string[] = []
    let resolveFirstStop: (() => void) | undefined
    let stops = 0

    const supervisor = new Supervisor({
      spawn: (target) => {
        events.push(`start ${target.name}`)
        return {
          async stop() {
            stops += 1
            events.push(`stop ${target.name}`)
            // The first stop hangs until released, so a second restart has to
            // wait rather than starting a third process alongside it.
            if (stops === 1) {
              await new Promise<void>((resolve) => {
                resolveFirstStop = resolve
              })
            }
          },
        }
      },
    })

    await supervisor.start(spec('server'))
    const first = supervisor.restart('server')
    const second = supervisor.restart('server')
    await new Promise((resolve) => setTimeout(resolve, 10))
    resolveFirstStop?.()
    await Promise.all([first, second])

    expect(events).toEqual([
      'start server',
      'stop server',
      'start server',
      'stop server',
      'start server',
    ])
  })

  it('does not let a failing stop poison later restarts', async () => {
    const events: string[] = []
    let failStop = true
    const supervisor = new Supervisor({
      spawn: (target) => {
        events.push(`start ${target.name}`)
        return {
          async stop() {
            if (failStop) {
              failStop = false
              throw new Error('already gone')
            }
            events.push(`stop ${target.name}`)
          },
        }
      },
    })

    await supervisor.start(spec('server'))
    // The stop throws; the restart must still start a new process.
    await supervisor.restart('server')
    await supervisor.restart('server')

    expect(events).toEqual([
      'start server',
      'start server',
      'stop server',
      'start server',
    ])
  })

  it('stopping something that was never started is a no-op', async () => {
    const { events, spawn } = recorder()
    const supervisor = new Supervisor({ spawn })
    await supervisor.stop('ghost')
    await supervisor.stopAll()
    expect(events).toEqual([])
  })

  it('logs what it is doing', async () => {
    const messages: string[] = []
    const { spawn } = recorder()
    const supervisor = new Supervisor({ spawn, log: (m) => messages.push(m) })

    await supervisor.start(spec('server'))
    await supervisor.stopAll()

    expect(messages).toEqual([
      'next-rs: starting server',
      'next-rs: stopping server',
    ])
  })
})
