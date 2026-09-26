import { usePipelineStore } from "../state/pipelineStore";
import type { ClientMessage, ServerMessage } from "../state/types";

export const SERVER_HTTP = "http://localhost:4173";
const SERVER_WS = "ws://localhost:4173/api/watch";

let socket: WebSocket | null = null;

export function disconnect(): void {
  socket?.close();
  socket = null;
}

export function connectWatchFile(path: string): void {
  disconnect();
  const store = usePipelineStore.getState();
  store.reset();
  store.setConnecting(true);
  store.setWatchedPath(path);

  const ws = new WebSocket(SERVER_WS);
  socket = ws;

  ws.onopen = () => {
    usePipelineStore.getState().setConnecting(false);
    usePipelineStore.getState().setConnected(true);
    const msg: ClientMessage = { kind: "watch_file", path };
    ws.send(JSON.stringify(msg));
  };
  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data as string) as ServerMessage;
    usePipelineStore.getState().handleMessage(msg);
  };
  ws.onclose = () => {
    usePipelineStore.getState().setConnected(false);
    usePipelineStore.getState().setConnecting(false);
  };
  ws.onerror = () => {
    usePipelineStore.getState().setConnecting(false);
  };
}
