;(function () {
  const invokeKey = __INVOKE_KEY__
  const internals = window.__TAURI_INTERNALS__
  let leaving = false
  window.addEventListener('pagehide', event => { leaving = !event.persisted })
  window.addEventListener('pageshow', () => { leaving = false })

  function encoded(payload) {
    if (payload instanceof ArrayBuffer || ArrayBuffer.isView(payload)) return { contentType: 'application/octet-stream', data: payload }
    if (Array.isArray(payload)) return { contentType: 'application/octet-stream', data: Uint8Array.from(payload) }
    const data = JSON.stringify(payload, (_key, value) => {
      if (value instanceof Map) return Object.fromEntries(value.entries())
      if (value instanceof Uint8Array) return Array.from(value)
      if (value instanceof ArrayBuffer) return Array.from(new Uint8Array(value))
      if (typeof value === 'object' && value !== null && '__TAURI_TO_IPC_KEY__' in value) return value.__TAURI_TO_IPC_KEY__()
      return value
    })
    return { contentType: 'application/json', data }
  }

  function replied(response, callback, error) {
    const id = response.headers.get('Tauri-Response') === 'ok' ? callback : error
    switch ((response.headers.get('content-type') || '').split(',')[0]) {
      case 'application/json': return response.json().then(value => [id, value])
      case 'text/plain': return response.text().then(value => [id, value])
      default: return response.arrayBuffer().then(value => [id, value])
    }
  }

  function send({ cmd, callback, error, payload, options }) {
    const { contentType, data } = encoded(payload)
    const headers = new Headers((options && options.headers) || {})
    headers.set('Content-Type', contentType)
    headers.set('Tauri-Callback', callback)
    headers.set('Tauri-Error', error)
    headers.set('Tauri-Invoke-Key', invokeKey)
    fetch(internals.convertFileSrc(cmd, 'ipc'), { method: 'POST', body: data, headers })
      .then(response => replied(response, callback, error))
      .then(
        ([id, value]) => internals.runCallback(id, value),
        failure => {
          if (leaving) return
          const message = `BattlePlan couldn’t reach its desktop host for ${cmd}: the IPC request failed (${failure}).`
          console.error(message)
          internals.runCallback(error, message)
        },
      )
  }

  Object.defineProperty(internals, 'postMessage', { value: send })
})()
