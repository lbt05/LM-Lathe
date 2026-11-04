/**
 * Typed client for the TTS HTTP API.
 *
 * The desktop application starts the `tts-server` sidecar and receives its
 * base URL from the Rust side; every request then goes straight to that
 * local server.
 */

export interface Model {
  id: string;
  name: string;
  family: string;
  engine_id: string;
  hf_repo: string;
  version: string;
  format: string;
  size_bytes: number;
  estimated_memory_bytes: number;
  features: string[];
  compatible: boolean;
  compatibility: string;
  notes: string[];
  installed: boolean;
  path: string | null;
  state: string;
  pinned: boolean;
}

export interface Voice {
  id: string;
  name: string;
  description: string;
  language: string | null;
  gender_presentation: string | null;
  age_style: string | null;
  personality: string | null;
  engine: string;
  model_id: string;
  engine_config: Record<string, unknown>;
  samples: string[];
  created_at: string;
  updated_at: string;
}

export interface VoiceCreation {
  voice: Voice;
  sample: string | null;
  sample_path: string | null;
}

export interface Tenant {
  key: string;
  kind: string;
  pinned: boolean;
  active_requests: number;
  estimated_bytes: number;
  reserved_bytes: number;
  last_used_at: string;
  ttl_secs: number;
  state: string;
}

export interface Scheduler {
  total_bytes: number;
  reserved_bytes: number;
  used_bytes: number;
  available_bytes: number;
  tenants: Tenant[];
}

export interface Hardware {
  platform: string;
  architecture: string;
  apple_silicon: boolean;
  cuda_available: boolean;
  nvidia_gpu: string | null;
  total_ram_bytes: number;
}

export interface Runtime {
  model_id: string;
  engine_id: string;
  state: string;
  pinned: boolean;
  process_id: number | null;
  endpoint: string | null;
  last_used_at: string | null;
  active_requests: number;
  estimated_memory: number;
  actual_memory: number;
}

export interface Status {
  runtimes: Runtime[];
  active_model: string | null;
  scheduler: Scheduler;
  hardware: Hardware;
}

export interface InstallProgress {
  model_id: string;
  state: string;
  files_done: number;
  files_total: number;
  bytes: number;
  error?: string;
}

/** The error the API answers with. */
export class ApiError extends Error {
  readonly kind: string;
  readonly status: number;

  constructor(kind: string, message: string, status: number) {
    super(message);
    this.kind = kind;
    this.status = status;
  }
}

let baseUrl = "";

export function setBaseUrl(url: string) {
  baseUrl = url.replace(/\/$/, "");
}

export function getBaseUrl() {
  return baseUrl;
}

/** Turns a byte count into a human readable size. */
export function formatBytes(bytes: number): string {
  if (!bytes) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const exponent = Math.min(
    units.length - 1,
    Math.floor(Math.log(bytes) / Math.log(1024)),
  );
  return `${(bytes / Math.pow(1024, exponent)).toFixed(exponent === 0 ? 0 : 1)} ${units[exponent]}`;
}

/** Formats an ISO timestamp for the table. */
export function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString("zh-CN", { hour12: false });
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  if (!baseUrl) {
    throw new ApiError("no_server", "后端服务尚未启动", 0);
  }
  const response = await fetch(`${baseUrl}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
  });

  if (!response.ok) {
    let kind = "unknown";
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = await response.json();
      kind = body.kind ?? kind;
      message = body.message ?? message;
    } catch {
      // the body was not JSON
    }
    throw new ApiError(kind, message, response.status);
  }

  if (response.status === 204) return undefined as T;
  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

export const api = {
  health: () => request<{ status: string; active_model: string | null }>("/health"),
  status: () => request<Status>("/v1/tts/status"),
  models: () => request<Model[]>("/v1/tts/models"),

  loadModel: (id: string) =>
    request<Runtime>(`/v1/tts/models/${id}/load`, { method: "POST" }),
  unloadModel: (id: string) =>
    request<void>(`/v1/tts/models/${id}/unload`, { method: "POST" }),
  pinModel: (id: string, pinned: boolean) =>
    request<void>(`/v1/tts/models/${id}/${pinned ? "pin" : "unpin"}`, {
      method: "POST",
    }),
  installModel: (id: string) =>
    request<void>(`/v1/tts/models/${id}/install`, { method: "POST" }),
  uninstallModel: (id: string) =>
    request<void>(`/v1/tts/models/${id}/uninstall`, { method: "POST" }),
  installProgress: (id: string) =>
    request<InstallProgress>(`/v1/tts/models/${id}/install`),
  modelLogs: async (id: string) => {
    const response = await fetch(`${baseUrl}/v1/tts/models/${id}/logs`);
    return response.text();
  },

  voices: (query: { model?: string; language?: string; query?: string } = {}) => {
    const params = new URLSearchParams();
    if (query.model) params.set("model", query.model);
    if (query.language) params.set("language", query.language);
    if (query.query) params.set("query", query.query);
    const suffix = params.toString();
    return request<Voice[]>(`/v1/tts/voices${suffix ? `?${suffix}` : ""}`);
  },
  voice: (id: string) => request<Voice>(`/v1/tts/voices/${id}`),
  updateVoice: (id: string, patch: Record<string, unknown>) =>
    request<Voice>(`/v1/tts/voices/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  deleteVoice: (id: string) =>
    request<void>(`/v1/tts/voices/${id}`, { method: "DELETE" }),

  designVoice: (body: Record<string, unknown>) =>
    request<VoiceCreation>("/v1/tts/voices/design", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  cloneVoice: (body: Record<string, unknown>) =>
    request<VoiceCreation>("/v1/tts/voices/clone", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  /** Synthesises speech and returns the raw audio. */
  speak: async (body: Record<string, unknown>): Promise<Blob> => {
    const response = await fetch(`${baseUrl}/v1/tts/speak`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      let kind = "unknown";
      let message = `${response.status} ${response.statusText}`;
      try {
        const payload = await response.json();
        kind = payload.kind ?? kind;
        message = payload.message ?? message;
      } catch {
        // the body was not JSON
      }
      throw new ApiError(kind, message, response.status);
    }
    return response.blob();
  },

  /** The URL of a generated sample. */
  sampleUrl: (voiceId: string, sample: string) =>
    `${baseUrl}/v1/tts/voices/${voiceId}/samples/${sample}`,
};
