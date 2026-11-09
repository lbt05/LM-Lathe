/**
 * Dragging of the window.
 *
 * macOS does not let the webview hand the drag over to the window. The
 * native drag of the system needs the mouse-down event itself, and by the
 * time the request of the interface reaches the Rust side the event is
 * gone — the window then refuses to move (tauri#9503).
 *
 * The window therefore follows the mouse on its own: the interface asks
 * where the window is when the drag starts, remembers where the mouse went
 * down, and the Rust side moves the window to the place the mouse is at.
 * The movement is sent once per frame, and only one request is on its way
 * at a time, so the window does not lag behind the mouse.
 */
import { invoke } from "@tauri-apps/api/core";
import type { PointerEvent as ReactPointerEvent } from "react";

/** Where the window is when the drag starts. */
interface DragOrigin {
  /** The position of the window, in physical pixels. */
  x: number;
  /** The position of the window, in physical pixels. */
  y: number;
  /** The ratio between the physical pixels and the points of the screen. */
  scale: number;
}

/** `true` when the frontend runs inside the Tauri webview. */
function inTauri() {
  return "__TAURI_INTERNALS__" in window;
}

/** The elements that keep their clicks instead of dragging the window. */
const CLICKABLE = "button, input, select, textarea, a, [role='button']";

/** The moment and the place of the last pointer down, for the double click. */
let lastDown = 0;
let lastX = 0;
let lastY = 0;

/**
 * Starts a drag of the window.
 *
 * This is the `onPointerDown` of every place that may drag the window. The
 * buttons and the input fields keep their clicks: they are recognised here
 * and the drag does not start.
 */
export function beginWindowDrag(event: ReactPointerEvent<Element>) {
  if (event.button !== 0 || !inTauri()) return;

  // A double click on the bar zooms the window, as it does everywhere else
  // on macOS.
  const now = Date.now();
  const double =
    now - lastDown < 400 &&
    Math.abs(event.screenX - lastX) < 5 &&
    Math.abs(event.screenY - lastY) < 5;
  lastDown = now;
  lastX = event.screenX;
  lastY = event.screenY;
  if (double) {
    void invoke("toggle_zoom");
    return;
  }

  // A click on a button or on a field belongs to that element.
  const hit = event.target as HTMLElement | null;
  if (hit?.closest?.(CLICKABLE)) return;

  const target = event.currentTarget as HTMLElement;
  // The pointer is caught, so that the drag keeps going when the mouse
  // leaves the window.
  try {
    target.setPointerCapture(event.pointerId);
  } catch {
    // The browser does not know the method; the drag stays in the window.
  }
  event.preventDefault();

  const pointerId = event.pointerId;
  const screenX = event.screenX;
  const screenY = event.screenY;
  let origin: DragOrigin | null = null;
  let mouse: { x: number; y: number } | null = null;
  let frame = 0;
  let moving = false;

  const tick = () => {
    frame = requestAnimationFrame(tick);
    if (!origin || !mouse || moving) return;
    moving = true;
    const x = Math.round(origin.x + (mouse.x - screenX) * origin.scale);
    const y = Math.round(origin.y + (mouse.y - screenY) * origin.scale);
    void invoke("drag_window_to", { x, y }).finally(() => {
      moving = false;
    });
  };

  const move = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    mouse = { x: ev.screenX, y: ev.screenY };
  };

  const stop = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    target.removeEventListener("pointermove", move);
    target.removeEventListener("pointerup", stop);
    target.removeEventListener("pointercancel", stop);
    if (frame) cancelAnimationFrame(frame);
    origin = null;
    mouse = null;
  };

  target.addEventListener("pointermove", move);
  target.addEventListener("pointerup", stop);
  target.addEventListener("pointercancel", stop);
  frame = requestAnimationFrame(tick);

  void invoke<DragOrigin>("begin_drag").then((value) => {
    origin = value;
  });
}
