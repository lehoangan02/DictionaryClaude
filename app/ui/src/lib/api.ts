// One entry point for every backend command. Inside the Tauri shell it calls
// the in-process Rust API; in a browser it talks to gredict-server over HTTP.
type Tauri = { invoke: (cmd: string, args: unknown) => Promise<unknown> };
const tauri = (window as unknown as { __TAURI_INTERNALS__?: Tauri }).__TAURI_INTERNALS__;

export async function call<T = any>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (tauri) {
    try {
      return (await tauri.invoke('call', { cmd, args })) as T;
    } catch (e) {
      throw new Error(typeof e === 'string' ? e : (e as Error).message);
    }
  }
  const r = await fetch(`/api/${cmd}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(args),
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error ?? r.statusText);
  return j as T;
}

/** Calls a command only the desktop shell has (native sound); null in a plain browser. */
export async function native<T = unknown>(cmd: string, args: Record<string, unknown> = {}): Promise<T | null> {
  if (!tauri) return null;
  return (await tauri.invoke(cmd, args)) as T;
}

export const isDesktopApp = !!tauri && !/Android/i.test(navigator.userAgent);
