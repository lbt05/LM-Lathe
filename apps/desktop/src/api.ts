/**
 * Typed client for the TTS HTTP API.
 *
 * The desktop application starts the `lm-lathe-server` sidecar and receives its
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
  parameters?: number | null;
  estimated_memory_bytes: number;
  features: string[];
  llm_features?: string[];
  compatible: boolean;
  compatibility: string;
  notes: string[];
  /** Things that are known to go wrong with this model on this machine. */
  warnings?: string[];
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
  available_ram_bytes?: number;
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
  actual_memory: number | null;
}

export interface Status {
  runtimes: Runtime[];
  llm_runtimes?: Runtime[];
  active_model: string | null;
  scheduler: Scheduler;
  hardware: Hardware;
}

export interface Route {
  method: string;
  path: string;
  summary: string;
}

/** The directories of the local service and its download settings. */
export interface Paths {
  root: string;
  tts_models_root: string;
  llm_models_root: string;
  runtimes_root: string;
  engines_root: string;
  voices_root: string;
  logs_root: string;
  download_concurrency: number;
  available_bytes: number;
}

/** A model of the Hugging Face Hub that the runtime can serve. */
export interface DiscoveredModel {
  id: string;
  repo: string;
  name: string;
  family: string;
  format: string;
  engine_id: string;
  downloads: number;
  likes: number;
  last_modified: string | null;
  license: string | null;
  quant: string | null;
  files: string[];
  installed: boolean;
  /** The size of the download in bytes. */
  size_bytes?: number | null;
  /** The number of parameters of the model. */
  parameters?: number | null;
  /** Things that are known to go wrong with this model on this machine. */
  warnings?: string[];
}

/** The answer of a search of the hub. */
export interface DiscoveredModels {
  models: DiscoveredModel[];
  /** The repositories that were found but dropped, because the runtime of
   *  the application cannot serve them. */
  dropped: number;
}

export interface InstallProgress {
  model_id: string;
  state: string;
  files_done: number;
  files_total: number;
  bytes: number;
  bytes_total: number;
  error?: string;
}

/** One installed version of an inference engine. */
export interface InstalledEngine {
  version: string;
  path: string;
  entry: string | null;
  installed_at: string;
  size_bytes: number;
  source: string;
}

/** An inference engine the manager knows about. */
export interface EngineAsset {
  id: string;
  name: string;
  summary: string;
  distribution: string;
  repository: string | null;
  package: string | null;
  program: string;
  programs: string[];
  note: string | null;
  requirements: string[];
  capabilities: string[];
  platforms: string[];
  supported_here: boolean;
  platform: string;
  system_entry: string | null;
  current_version: string | null;
  installed: InstalledEngine[];
  size_bytes: number;
  status: string;
  model_engine_ids: string[];
}

/** A version of an engine that can be installed. */
export interface EngineVersion {
  version: string;
  published_at: string | null;
  prerelease: boolean;
  size_bytes: number | null;
  asset_name: string | null;
  download_url: string | null;
  installed: boolean;
}

/** Progress of a running engine installation. */
export interface EngineInstallProgress {
  engine_id: string;
  version: string;
  state: string;
  percent: number | null;
  bytes: number;
  total_bytes: number | null;
  message: string;
  log: string[];
  error?: string;
}

/** The result of an engine health check. */
export interface EngineHealth {
  engine_id: string;
  status: string;
  detail: string;
  entry: string | null;
  endpoint: string | null;
  duration_ms: number;
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
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined) return "—";
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
      // Die Art des Fehlers steht schon im Text, er steht nur hindersaft.
      message = message.replace(/^invalid request:\s*/, "");
    } catch {
      // the body was not JSON
    }
    throw new ApiError(kind, message, response.status);
  }

  if (response.status === 204) return undefined as T;
  const text = await response.text();
  if (!text) return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    // Some actions answer with a plain text message, e.g. `the
    // installation has started` with the status 202.
    return text as unknown as T;
  }
}

export const api = {
  health: () => request<{ status: string; active_model: string | null }>("/health"),
  status: () => request<Status>("/v1/tts/status"),
  models: () => request<Model[]>("/v1/tts/models"),
  routes: () => request<Route[]>("/v1/routes"),
  /** The directories of the local service and its download settings. */
  paths: () => request<Paths>("/v1/paths"),
  /** Sets how many files of a repository are fetched at the same time. */
  setDownloadConcurrency: (concurrency: number) =>
    request<{ concurrency: number }>("/v1/settings/download-concurrency", {
      method: "POST",
      body: JSON.stringify({ concurrency }),
    }),

  /** Searches the Hugging Face Hub for models the runtime can serve. */
  discover: async (query: string, limit = 20) => {
    try {
      return await request<DiscoveredModels>(
        `/v1/tts/discover?q=${encodeURIComponent(query)}&limit=${limit}`,
      );
    } catch (err) {
      // The route is new; a backend that does not know it is an older build.
      if (err instanceof ApiError && err.status === 404) {
        throw new ApiError(
          "not_found",
          "本地服务太旧，没有搜索接口。重启应用就会更新它。",
          err.status,
        );
      }
      throw err;
    }
  },
  /** Downloads a model that was found by the search. */
  installDiscovered: (repo: string) =>
    request<{ model_id: string; name: string }>("/v1/tts/discover/install", {
      method: "POST",
      body: JSON.stringify({ repo }),
    }),

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
    request<void>(`/v1/tts/models/${id}/uninstall`, {
      method: "POST",
      body: JSON.stringify({ confirm: true }),
    }),
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

  /** The inference engines and their installation state. */
  engines: () => request<EngineAsset[]>("/v1/runtimes"),
  /** The versions of an engine that can be installed. */
  engineVersions: (id: string) =>
    request<EngineVersion[]>(`/v1/runtimes/${id}/versions`),
  /** Starts the installation of an engine version. */
  installEngine: (id: string, version?: string) =>
    request<void>(`/v1/runtimes/${id}/install`, {
      method: "POST",
      body: JSON.stringify({ version: version ?? null }),
    }),
  /** The progress of a running engine installation. */
  engineInstallProgress: (id: string) =>
    request<EngineInstallProgress>(`/v1/runtimes/${id}/install-progress`),
  /** Activates an installed engine version. */
  activateEngine: (id: string, version: string) =>
    request<void>(`/v1/runtimes/${id}/activate`, {
      method: "POST",
      body: JSON.stringify({ version }),
    }),
  /** Removes an installed engine version. */
  uninstallEngine: (id: string, version: string) =>
    request<void>(`/v1/runtimes/${id}/uninstall`, {
      method: "POST",
      body: JSON.stringify({ version }),
    }),
  /** Starts the engine service and probes its health endpoint. */
  engineHealth: (id: string) =>
    request<EngineHealth>(`/v1/runtimes/${id}/health`, { method: "POST" }),

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
