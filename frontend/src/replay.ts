// WebSocket replay client. One socket per open session; closes and
// reopens whenever the selected session changes. Cursor updates land on
// the Zustand store so any component can subscribe.
//
// Connection lifecycle is deliberately simple for M3: open → replay_open
// → controls → close. M5 will re-use the same socket for live tailing.

import { useUi } from "./store";

let ws: WebSocket | null = null;
let currentSession: string | null = null;

type ServerMsg =
  | { kind: "replay_bounds"; from_ts: number; to_ts: number; speed: number; playing: boolean }
  | { kind: "cursor"; ts_ms: number }
  | { kind: "error"; message: string }
  | { kind: "pong" };

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/ws`;
}

function send(obj: unknown) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(obj));
  }
}

export function openReplay(sessionId: string) {
  if (currentSession === sessionId && ws && ws.readyState === WebSocket.OPEN) {
    return;
  }
  closeReplay();
  currentSession = sessionId;
  const sock = new WebSocket(wsUrl());
  ws = sock;
  sock.onopen = () => {
    send({ op: "replay_open", session_id: sessionId });
  };
  sock.onmessage = (ev) => {
    let msg: ServerMsg;
    try {
      msg = JSON.parse(ev.data);
    } catch {
      return;
    }
    const ui = useUi.getState();
    switch (msg.kind) {
      case "replay_bounds":
        ui.setBounds(msg.from_ts, msg.to_ts);
        ui.setPlaying(msg.playing);
        ui.setSpeed(msg.speed);
        break;
      case "cursor":
        ui.setCursor(msg.ts_ms);
        break;
      case "error":
        useUi.setState({ replayError: msg.message });
        break;
    }
  };
  sock.onclose = () => {
    if (ws === sock) ws = null;
  };
  sock.onerror = () => {
    useUi.setState({ replayError: "websocket error" });
  };
}

export function closeReplay() {
  if (ws) {
    try {
      ws.close();
    } catch {
      /* noop */
    }
    ws = null;
  }
  currentSession = null;
}

export const replay = {
  play: () => send({ op: "replay_control", action: "play" }),
  pause: () => send({ op: "replay_control", action: "pause" }),
  seek: (ts_ms: number) => send({ op: "replay_control", action: "seek", value: ts_ms }),
  speed: (mult: number) => send({ op: "replay_control", action: "speed", value: mult }),
};
