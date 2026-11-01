// SDK mínimo para overlays de streams-tools.
(() => {
  const listeners = new Map();
  let retryMs = 1000;

  function dispatch(event) {
    // Cada callback recibe los datos y, como segundo argumento, el evento completo.
    for (const fn of listeners.get(event.type) || []) fn(event.data, event);
    for (const fn of listeners.get("*") || []) fn(event.data, event);
  }

  function connect() {
    const socket = new WebSocket(`ws://${location.host}/ws`);
    socket.onopen = () => {
      retryMs = 1000;
    };
    socket.onmessage = ({ data }) => {
      try {
        dispatch(JSON.parse(data));
      } catch (_) {
        // Ignora mensajes no JSON y errores de callbacks del overlay.
      }
    };
    socket.onerror = () => socket.close();
    socket.onclose = () => {
      setTimeout(connect, retryMs);
      retryMs = Math.min(retryMs * 2, 10000);
    };
  }

  function on(type, fn) {
    if (!listeners.has(type)) listeners.set(type, new Set());
    listeners.get(type).add(fn);
    return () => listeners.get(type)?.delete(fn);
  }

  async function emit(type, data) {
    const event = {
      id: crypto.randomUUID(),
      platform: "plugin:overlay",
      type,
      user: { id: "overlay", name: "overlay", roles: [] },
      data,
      ts: new Date().toISOString(),
    };
    const response = await fetch("/api/events", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(event),
    });
    if (!response.ok) throw new Error(await response.text());
  }

  window.streams = { on, emit };
  connect();
})();
