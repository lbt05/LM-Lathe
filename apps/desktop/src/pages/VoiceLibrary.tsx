import { useCallback, useEffect, useState } from "react";
import { api, formatDate, type Model, type Voice } from "../api";

/**
 * Voice Library (§9.2).
 *
 * Lists the stored voice assets with their samples, and allows filtering by
 * model and language as well as editing and deleting a profile.
 */
export default function VoiceLibrary({ models }: { models: Model[] }) {
  const [voices, setVoices] = useState<Voice[]>([]);
  const [modelFilter, setModelFilter] = useState("");
  const [languageFilter, setLanguageFilter] = useState("");
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Voice | null>(null);
  const [draft, setDraft] = useState({
    name: "",
    description: "",
    language: "",
    personality: "",
    engineConfig: "{}",
  });

  const refresh = useCallback(async () => {
    try {
      const list = await api.voices({
        model: modelFilter || undefined,
        language: languageFilter || undefined,
        query: query || undefined,
      });
      setVoices(list);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [modelFilter, languageFilter, query]);

  useEffect(() => {
    const timer = window.setTimeout(() => void refresh(), 200);
    return () => window.clearTimeout(timer);
  }, [refresh]);

  const startEditing = (voice: Voice) => {
    setEditing(voice);
    setDraft({
      name: voice.name,
      description: voice.description,
      language: voice.language ?? "",
      personality: voice.personality ?? "",
      engineConfig: JSON.stringify(voice.engine_config ?? {}, null, 2),
    });
  };

  const save = async () => {
    if (!editing) return;
    let engineConfig: Record<string, unknown> = {};
    try {
      engineConfig = JSON.parse(draft.engineConfig || "{}");
    } catch (err) {
      setError(`引擎参数不是合法 JSON：${String(err)}`);
      return;
    }
    try {
      await api.updateVoice(editing.id, {
        name: draft.name,
        description: draft.description,
        language: draft.language || null,
        personality: draft.personality || null,
        engine_config: engineConfig,
      });
      setEditing(null);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const remove = async (voice: Voice) => {
    if (!window.confirm(`确定删除声音「${voice.name}」？`)) return;
    try {
      await api.deleteVoice(voice.id);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const languages = Array.from(
    new Set(voices.map((voice) => voice.language).filter(Boolean)),
  ) as string[];

  return (
    <section className="page">
      <header className="page-header">
        <div>
          <h2>声音库</h2>
          <p className="muted">
            Voice Profile 独立于推理进程，保存后长期可用。
          </p>
        </div>
      </header>

      <div className="filters">
        <label>
          模型
          <select
            value={modelFilter}
            onChange={(event) => setModelFilter(event.target.value)}
          >
            <option value="">全部模型</option>
            {models.map((model) => (
              <option key={model.id} value={model.id}>
                {model.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          语言
          <input
            list="voice-languages"
            value={languageFilter}
            onChange={(event) => setLanguageFilter(event.target.value)}
            placeholder="如 Chinese"
          />
          <datalist id="voice-languages">
            {languages.map((language) => (
              <option key={language} value={language} />
            ))}
          </datalist>
        </label>
        <label>
          搜索
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="名称或描述"
          />
        </label>
        <button onClick={() => void refresh()}>刷新</button>
      </div>

      {error && <div className="alert error">{error}</div>}

      {voices.length === 0 ? (
        <p className="muted">还没有声音资产，可以在「声音设计工作台」中创建。</p>
      ) : (
        <div className="voice-grid">
          {voices.map((voice) => (
            <article className="card" key={voice.id}>
              <header className="card-header">
                <div>
                  <h3>{voice.name}</h3>
                  <div className="muted small">
                    {voice.id} · {voice.model_id}
                  </div>
                </div>
              </header>
              <p className="description">{voice.description || "（没有描述）"}</p>
              <dl className="meta">
                <div>
                  <dt>语言</dt>
                  <dd>{voice.language ?? "—"}</dd>
                </div>
                <div>
                  <dt>人格</dt>
                  <dd>{voice.personality ?? "—"}</dd>
                </div>
                <div>
                  <dt>创建时间</dt>
                  <dd>{formatDate(voice.created_at)}</dd>
                </div>
              </dl>
              {voice.samples.length > 0 && (
                <div className="samples">
                  {voice.samples.map((sample) => (
                    <audio
                      key={sample}
                      controls
                      preload="none"
                      src={api.sampleUrl(voice.id, sample)}
                    />
                  ))}
                </div>
              )}
              <footer className="card-footer">
                <button onClick={() => startEditing(voice)}>编辑</button>
                <button className="danger" onClick={() => void remove(voice)}>
                  删除
                </button>
              </footer>
            </article>
          ))}
        </div>
      )}

      {editing && (
        <div className="modal-backdrop" onClick={() => setEditing(null)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <header className="modal-header">
              <h3>编辑声音 · {editing.name}</h3>
              <button onClick={() => setEditing(null)}>关闭</button>
            </header>
            <div className="form">
              <label>
                名称
                <input
                  value={draft.name}
                  onChange={(event) =>
                    setDraft({ ...draft, name: event.target.value })
                  }
                />
              </label>
              <label>
                描述
                <textarea
                  rows={3}
                  value={draft.description}
                  onChange={(event) =>
                    setDraft({ ...draft, description: event.target.value })
                  }
                />
              </label>
              <label>
                语言
                <input
                  value={draft.language}
                  onChange={(event) =>
                    setDraft({ ...draft, language: event.target.value })
                  }
                />
              </label>
              <label>
                人格
                <input
                  value={draft.personality}
                  onChange={(event) =>
                    setDraft({ ...draft, personality: event.target.value })
                  }
                />
              </label>
              <label>
                引擎参数（JSON）
                <textarea
                  rows={6}
                  spellCheck={false}
                  value={draft.engineConfig}
                  onChange={(event) =>
                    setDraft({ ...draft, engineConfig: event.target.value })
                  }
                />
              </label>
              <div className="form-actions">
                <button onClick={() => void save()}>保存</button>
                <button onClick={() => setEditing(null)}>取消</button>
              </div>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
